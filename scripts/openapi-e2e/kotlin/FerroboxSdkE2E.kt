import java.io.File
import java.util.Base64
import org.openapitools.client.apis.CommandsApi
import org.openapitools.client.apis.FilesApi
import org.openapitools.client.apis.SandboxesApi
import org.openapitools.client.infrastructure.ApiClient
import org.openapitools.client.infrastructure.ClientException
import org.openapitools.client.models.CreateSandboxRequest
import org.openapitools.client.models.ExecuteCommandRequest
import org.openapitools.client.models.NetworkRequest
import org.openapitools.client.models.SandboxState
import org.openapitools.client.models.WriteFileRequest

private val checks = listOf(
    "generated-model-create",
    "bearer-auth-inspect",
    "typed-command-execution",
    "lossless-base64-output",
    "typed-file-roundtrip",
    "delete-and-stale-handle-rejection",
    "credential-redaction",
)

private fun requiredEnvironment(name: String): String =
    requireNotNull(System.getenv(name)?.takeIf { it.isNotEmpty() }) { "$name is required" }

fun main() {
    val apiUrl = requiredEnvironment("FERROBOX_API_URL")
    val auditPath = File(requiredEnvironment("FERROBOX_AUDIT_LOG"))
    val evidencePath = File(requiredEnvironment("FERROBOX_OPENAPI_SDK_EVIDENCE"))

    ApiClient.accessToken = null
    val publicSandboxes = SandboxesApi(apiUrl)
    val created = publicSandboxes.createSandbox(
        CreateSandboxRequest(
            template = "python",
            cpuCount = 1,
            memoryMb = 512,
            timeoutSeconds = 120L,
            network = NetworkRequest(internetAccess = false),
        ),
    )
    check(created.state == SandboxState.running) { "created sandbox is not running" }
    check(created.token.isNotEmpty()) { "create response token is absent" }

    ApiClient.accessToken = created.token
    val sandboxes = SandboxesApi(apiUrl)
    val commands = CommandsApi(apiUrl)
    val files = FilesApi(apiUrl)
    var deleted = false
    try {
        val inspected = sandboxes.getSandbox(created.sandboxId)
        check(inspected.sandboxId == created.sandboxId) { "inspect returned another sandbox" }
        check(inspected.state == SandboxState.running) { "inspected sandbox is not running" }

        val executed = commands.executeCommand(
            created.sandboxId,
            ExecuteCommandRequest(
                argv = listOf("python3", "-c", "print(40 + 2)"),
                cwd = "/home/sandbox",
                environment = emptyMap(),
                timeoutSeconds = 30L,
                maxOutputBytes = 1048576L,
            ),
        )
        check(executed.stdout == "42\n") { "typed stdout mismatch" }
        check(String(Base64.getDecoder().decode(executed.stdoutBase64), Charsets.UTF_8) == "42\n") {
            "base64 stdout mismatch"
        }

        val payload = "generated-openapi-client\n".toByteArray(Charsets.UTF_8)
        val written = files.writeFile(
            created.sandboxId,
            WriteFileRequest(
                path = "/home/sandbox/openapi.txt",
                contentBase64 = Base64.getEncoder().encodeToString(payload),
                overwrite = false,
            ),
        )
        check(written.bytesWritten == payload.size.toLong()) { "written byte count mismatch" }

        val read = files.readFile(
            created.sandboxId,
            "/home/sandbox/openapi.txt",
            0L,
            1048576L,
        )
        check(Base64.getDecoder().decode(read.contentBase64).contentEquals(payload)) { "file content mismatch" }
        check(read.eof) { "file read did not reach EOF" }

        sandboxes.deleteSandbox(created.sandboxId)
        deleted = true
        val stale = runCatching { sandboxes.getSandbox(created.sandboxId) }.exceptionOrNull()
        check(stale is ClientException && stale.statusCode == 404) { "deleted sandbox remained addressable" }

        val audit = auditPath.readText(Charsets.UTF_8)
        check(!audit.contains(created.token)) { "bearer credential reached audit log" }
        check(audit.contains("\"operation\":\"delete\"")) { "delete audit is absent" }

        val serializedChecks = checks.joinToString(",\n") { "    \"$it\"" }
        val evidence = """
            {
              "schema_version": 1,
              "language": "kotlin",
              "sandbox_id": "${created.sandboxId}",
              "checks": [
            $serializedChecks
              ]
            }
        """.trimIndent() + "\n"
        evidencePath.writeText(evidence, Charsets.UTF_8)
        print(evidence)
    } finally {
        if (!deleted) {
            runCatching { sandboxes.deleteSandbox(created.sandboxId) }
        }
        ApiClient.accessToken = null
    }
}
