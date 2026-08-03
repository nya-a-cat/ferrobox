package org.openapitools.client;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Arrays;
import java.util.Base64;
import java.util.Collections;
import org.junit.jupiter.api.Test;
import org.openapitools.client.api.CommandsApi;
import org.openapitools.client.api.FilesApi;
import org.openapitools.client.api.SandboxesApi;
import org.openapitools.client.model.CreateSandboxRequest;
import org.openapitools.client.model.CreateSandboxResponse;
import org.openapitools.client.model.ExecuteCommandRequest;
import org.openapitools.client.model.ExecuteCommandResponse;
import org.openapitools.client.model.NetworkRequest;
import org.openapitools.client.model.ReadFileResponse;
import org.openapitools.client.model.SandboxResponse;
import org.openapitools.client.model.SandboxState;
import org.openapitools.client.model.WriteFileRequest;
import org.openapitools.client.model.WriteFileResponse;

public final class GeneratedClientE2E {
    private static final String[] CHECKS = {
        "generated-model-create",
        "bearer-auth-inspect",
        "typed-command-execution",
        "lossless-base64-output",
        "typed-file-roundtrip",
        "delete-and-stale-handle-rejection",
        "credential-redaction"
    };

    @Test
    public void generatedClientCompletesOwnedSandboxLifecycle() throws Exception {
        String apiUrl = requiredEnvironment("FERROBOX_API_URL");
        Path auditPath = Paths.get(requiredEnvironment("FERROBOX_AUDIT_LOG"));
        Path evidencePath = Paths.get(requiredEnvironment("FERROBOX_OPENAPI_SDK_EVIDENCE"));

        ApiClient client = new ApiClient().setBasePath(apiUrl);
        SandboxesApi sandboxes = new SandboxesApi(client);
        CommandsApi commands = new CommandsApi(client);
        FilesApi files = new FilesApi(client);

        CreateSandboxResponse created = sandboxes.createSandbox(
            new CreateSandboxRequest()
                .template("python")
                .cpuCount(1)
                .memoryMb(512)
                .timeoutSeconds(120L)
                .network(new NetworkRequest().internetAccess(false)));
        assertEquals(SandboxState.RUNNING, created.getState());
        assertNotNull(created.getSandboxId());
        assertFalse(created.getToken().isEmpty());

        client.setBearerToken(created.getToken());
        boolean deleted = false;
        try {
            SandboxResponse inspected = sandboxes.getSandbox(created.getSandboxId());
            assertEquals(created.getSandboxId(), inspected.getSandboxId());
            assertEquals(SandboxState.RUNNING, inspected.getState());

            ExecuteCommandResponse executed = commands.executeCommand(
                created.getSandboxId(),
                new ExecuteCommandRequest()
                    .argv(Arrays.asList("python3", "-c", "print(40 + 2)"))
                    .cwd("/home/sandbox")
                    .environment(Collections.<String, String>emptyMap())
                    .timeoutSeconds(30L)
                    .maxOutputBytes(1048576L));
            assertEquals("42\n", executed.getStdout());
            assertArrayEquals(
                "42\n".getBytes(StandardCharsets.UTF_8),
                Base64.getDecoder().decode(executed.getStdoutBase64()));

            byte[] payload = "generated-openapi-client\n".getBytes(StandardCharsets.UTF_8);
            WriteFileResponse written = files.writeFile(
                created.getSandboxId(),
                new WriteFileRequest()
                    .path("/home/sandbox/openapi.txt")
                    .contentBase64(Base64.getEncoder().encodeToString(payload))
                    .overwrite(false));
            assertEquals(Long.valueOf(payload.length), written.getBytesWritten());

            ReadFileResponse read = files.readFile(
                created.getSandboxId(),
                "/home/sandbox/openapi.txt",
                0L,
                1048576L);
            assertArrayEquals(payload, Base64.getDecoder().decode(read.getContentBase64()));
            assertTrue(read.getEof());

            sandboxes.deleteSandbox(created.getSandboxId());
            deleted = true;
            ApiException stale = assertThrows(
                ApiException.class,
                () -> sandboxes.getSandbox(created.getSandboxId()));
            assertEquals(404, stale.getCode());

            String audit = new String(Files.readAllBytes(auditPath), StandardCharsets.UTF_8);
            assertFalse(audit.contains(created.getToken()));
            assertTrue(audit.contains("\"operation\":\"delete\""));

            JsonObject evidence = new JsonObject();
            evidence.addProperty("schema_version", 1);
            evidence.addProperty("language", "java");
            evidence.addProperty("sandbox_id", created.getSandboxId().toString());
            JsonArray checks = new JsonArray();
            for (String check : CHECKS) {
                checks.add(check);
            }
            evidence.add("checks", checks);
            Gson gson = new GsonBuilder().setPrettyPrinting().create();
            String serialized = gson.toJson(evidence) + "\n";
            Files.write(evidencePath, serialized.getBytes(StandardCharsets.UTF_8));
            System.out.print(serialized);
        } finally {
            if (!deleted) {
                try {
                    sandboxes.deleteSandbox(created.getSandboxId());
                } catch (ApiException ignored) {
                    // The API process cleanup remains the final failure boundary.
                }
            }
        }
    }

    private static String requiredEnvironment(String name) {
        String value = System.getenv(name);
        if (value == null || value.isEmpty()) {
            throw new IllegalStateException(name + " is required");
        }
        return value;
    }
}
