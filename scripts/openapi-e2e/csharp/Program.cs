using System.Net;
using System.Text;
using System.Text.Json;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Ferrobox.Client.Api;
using Ferrobox.Client.Client;
using Ferrobox.Client.Extensions;
using Ferrobox.Client.Model;

internal static class Program
{
    private static readonly string[] Checks =
    [
        "generated-model-create",
        "bearer-auth-inspect",
        "typed-command-execution",
        "lossless-base64-output",
        "typed-file-roundtrip",
        "delete-and-stale-handle-rejection",
        "credential-redaction",
    ];

    private static IHost BuildHost(string apiUrl, string token) =>
        Host.CreateDefaultBuilder()
            .ConfigureLogging(logging => logging.ClearProviders())
            .ConfigureApi((_, _, options) =>
            {
                options.AddTokens(new BearerToken(token));
                options.UseProvider<RateLimitProvider<BearerToken>, BearerToken>();
                options.AddApiHttpClients(client => client.BaseAddress = new Uri(apiUrl));
            })
            .Build();

    private static void Require(bool condition, string message)
    {
        if (!condition)
        {
            throw new InvalidOperationException(message);
        }
    }

    public static async Task Main()
    {
        var apiUrl = Environment.GetEnvironmentVariable("FERROBOX_API_URL")
            ?? throw new InvalidOperationException("FERROBOX_API_URL is required");
        var auditPath = Environment.GetEnvironmentVariable("FERROBOX_AUDIT_LOG")
            ?? throw new InvalidOperationException("FERROBOX_AUDIT_LOG is required");
        var evidencePath = Environment.GetEnvironmentVariable("FERROBOX_OPENAPI_SDK_EVIDENCE")
            ?? throw new InvalidOperationException("FERROBOX_OPENAPI_SDK_EVIDENCE is required");

        Guid sandboxId;
        string token;
        using (var publicHost = BuildHost(apiUrl, "public-bootstrap"))
        {
            var sandboxes = publicHost.Services.GetRequiredService<ISandboxesApi>();
            var response = await sandboxes.CreateSandboxAsync(
                new CreateSandboxRequest(
                    "python",
                    cpuCount: 1,
                    memoryMb: 512,
                    timeoutSeconds: 120L,
                    network: new NetworkRequest(false)));
            Require(response.IsCreated, $"create returned {(int)response.StatusCode}");
            var created = response.Created()
                ?? throw new InvalidOperationException("create response body is absent");
            Require(created.State == SandboxState.Running, "created sandbox is not running");
            sandboxId = created.SandboxId;
            token = created.Token;
            Require(!string.IsNullOrEmpty(token), "create response token is absent");
        }

        var deleted = false;
        using var authenticatedHost = BuildHost(apiUrl, token);
        var ownedSandboxes = authenticatedHost.Services.GetRequiredService<ISandboxesApi>();
        try
        {
            var commands = authenticatedHost.Services.GetRequiredService<ICommandsApi>();
            var files = authenticatedHost.Services.GetRequiredService<IFilesApi>();

            var inspectedResponse = await ownedSandboxes.GetSandboxAsync(sandboxId);
            Require(inspectedResponse.IsOk, $"inspect returned {(int)inspectedResponse.StatusCode}");
            var inspected = inspectedResponse.Ok()
                ?? throw new InvalidOperationException("inspect response body is absent");
            Require(inspected.SandboxId == sandboxId, "inspect returned another sandbox");
            Require(inspected.State == SandboxState.Running, "inspected sandbox is not running");

            var executedResponse = await commands.ExecuteCommandAsync(
                sandboxId,
                new ExecuteCommandRequest(
                    ["python3", "-c", "print(40 + 2)"],
                    cwd: "/home/sandbox",
                    varEnvironment: new Dictionary<string, string>(),
                    timeoutSeconds: 30L,
                    maxOutputBytes: 1048576L));
            Require(executedResponse.IsOk, $"execute returned {(int)executedResponse.StatusCode}");
            var executed = executedResponse.Ok()
                ?? throw new InvalidOperationException("execute response body is absent");
            Require(executed.Stdout == "42\n", "typed stdout mismatch");
            Require(
                Encoding.UTF8.GetString(Convert.FromBase64String(executed.StdoutBase64)) == "42\n",
                "base64 stdout mismatch");

            var payload = Encoding.UTF8.GetBytes("generated-openapi-client\n");
            var writeResponse = await files.WriteFileAsync(
                sandboxId,
                new WriteFileRequest(
                    "/home/sandbox/openapi.txt",
                    Convert.ToBase64String(payload),
                    overwrite: false));
            Require(writeResponse.IsOk, $"write returned {(int)writeResponse.StatusCode}");
            var written = writeResponse.Ok()
                ?? throw new InvalidOperationException("write response body is absent");
            Require(written.BytesWritten == payload.LongLength, "written byte count mismatch");

            var readResponse = await files.ReadFileAsync(
                sandboxId,
                "/home/sandbox/openapi.txt",
                offset: 0L,
                maxBytes: 1048576L);
            Require(readResponse.IsOk, $"read returned {(int)readResponse.StatusCode}");
            var read = readResponse.Ok()
                ?? throw new InvalidOperationException("read response body is absent");
            Require(Convert.FromBase64String(read.ContentBase64).SequenceEqual(payload), "file content mismatch");
            Require(read.Eof, "file read did not reach EOF");

            var deleteResponse = await ownedSandboxes.DeleteSandboxAsync(sandboxId);
            Require(deleteResponse.IsNoContent, $"delete returned {(int)deleteResponse.StatusCode}");
            deleted = true;
            var stale = await ownedSandboxes.GetSandboxAsync(sandboxId);
            Require(stale.StatusCode == HttpStatusCode.NotFound, "deleted sandbox remained addressable");

            var audit = await File.ReadAllTextAsync(auditPath, Encoding.UTF8);
            Require(!audit.Contains(token, StringComparison.Ordinal), "bearer credential reached audit log");
            Require(audit.Contains("\"operation\":\"delete\"", StringComparison.Ordinal), "delete audit is absent");

            var evidence = new
            {
                schema_version = 1,
                language = "csharp",
                sandbox_id = sandboxId.ToString(),
                checks = Checks,
            };
            await File.WriteAllTextAsync(
                evidencePath,
                JsonSerializer.Serialize(evidence, new JsonSerializerOptions { WriteIndented = true }) + "\n",
                new UTF8Encoding(encoderShouldEmitUTF8Identifier: false));
            Console.WriteLine(JsonSerializer.Serialize(evidence));
        }
        finally
        {
            if (!deleted)
            {
                try
                {
                    await ownedSandboxes.DeleteSandboxAsync(sandboxId);
                }
                catch
                {
                    // The API process cleanup remains the final failure boundary.
                }
            }
        }
    }
}
