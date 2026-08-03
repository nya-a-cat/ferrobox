package main

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"strings"

	openapi "github.com/GIT_USER_ID/GIT_REPO_ID"
)

var checks = []string{
	"generated-model-create",
	"bearer-auth-inspect",
	"typed-command-execution",
	"lossless-base64-output",
	"typed-file-roundtrip",
	"delete-and-stale-handle-rejection",
	"credential-redaction",
}

type evidence struct {
	SchemaVersion int      `json:"schema_version"`
	Language      string   `json:"language"`
	SandboxID     string   `json:"sandbox_id"`
	Checks        []string `json:"checks"`
}

func require(condition bool, message string) {
	if !condition {
		panic(message)
	}
}

func main() {
	apiURL := os.Getenv("FERROBOX_API_URL")
	auditPath := os.Getenv("FERROBOX_AUDIT_LOG")
	evidencePath := os.Getenv("FERROBOX_OPENAPI_SDK_EVIDENCE")
	require(apiURL != "" && auditPath != "" && evidencePath != "", "required environment is absent")

	configuration := openapi.NewConfiguration()
	configuration.Servers[0].URL = apiURL
	client := openapi.NewAPIClient(configuration)

	network := openapi.NewNetworkRequest()
	network.SetInternetAccess(false)
	createRequest := openapi.NewCreateSandboxRequest("python")
	createRequest.SetCpuCount(1)
	createRequest.SetMemoryMb(512)
	createRequest.SetTimeoutSeconds(120)
	createRequest.SetNetwork(*network)
	created, response, err := client.SandboxesAPI.CreateSandbox(context.Background()).
		CreateSandboxRequest(*createRequest).
		Execute()
	require(err == nil, fmt.Sprintf("create failed: %v", err))
	require(response.StatusCode == 201, fmt.Sprintf("create returned %d", response.StatusCode))
	require(created.GetState() == openapi.RUNNING, "created sandbox is not running")

	sandboxID := created.GetSandboxId()
	token := created.GetToken()
	require(sandboxID != "" && token != "", "create identity is absent")
	ownedContext := context.WithValue(context.Background(), openapi.ContextAccessToken, token)
	deleted := false
	defer func() {
		if !deleted {
			_, _ = client.SandboxesAPI.DeleteSandbox(ownedContext, sandboxID).Execute()
		}
	}()

	inspected, response, err := client.SandboxesAPI.GetSandbox(ownedContext, sandboxID).Execute()
	require(err == nil, fmt.Sprintf("inspect failed: %v", err))
	require(response.StatusCode == 200, fmt.Sprintf("inspect returned %d", response.StatusCode))
	require(inspected.GetSandboxId() == sandboxID, "inspect returned another sandbox")
	require(inspected.GetState() == openapi.RUNNING, "inspected sandbox is not running")

	executeRequest := openapi.NewExecuteCommandRequest([]string{"python3", "-c", "print(40 + 2)"})
	executeRequest.SetCwd("/home/sandbox")
	executeRequest.SetEnvironment(map[string]string{})
	executeRequest.SetTimeoutSeconds(30)
	executeRequest.SetMaxOutputBytes(1048576)
	executed, response, err := client.CommandsAPI.ExecuteCommand(ownedContext, sandboxID).
		ExecuteCommandRequest(*executeRequest).
		Execute()
	require(err == nil, fmt.Sprintf("execute failed: %v", err))
	require(response.StatusCode == 200, fmt.Sprintf("execute returned %d", response.StatusCode))
	require(executed.GetStdout() == "42\n", "typed stdout mismatch")
	decodedStdout, err := base64.StdEncoding.DecodeString(executed.GetStdoutBase64())
	require(err == nil && string(decodedStdout) == "42\n", "base64 stdout mismatch")

	payload := []byte("generated-openapi-client\n")
	writeRequest := openapi.NewWriteFileRequest(
		"/home/sandbox/openapi.txt",
		base64.StdEncoding.EncodeToString(payload),
	)
	writeRequest.SetOverwrite(false)
	written, response, err := client.FilesAPI.WriteFile(ownedContext, sandboxID).
		WriteFileRequest(*writeRequest).
		Execute()
	require(err == nil, fmt.Sprintf("write failed: %v", err))
	require(response.StatusCode == 200, fmt.Sprintf("write returned %d", response.StatusCode))
	require(written.GetBytesWritten() == int64(len(payload)), "written byte count mismatch")

	read, response, err := client.FilesAPI.ReadFile(ownedContext, sandboxID).
		Path("/home/sandbox/openapi.txt").
		Offset(0).
		MaxBytes(1048576).
		Execute()
	require(err == nil, fmt.Sprintf("read failed: %v", err))
	require(response.StatusCode == 200, fmt.Sprintf("read returned %d", response.StatusCode))
	content, err := base64.StdEncoding.DecodeString(read.GetContentBase64())
	require(err == nil && bytes.Equal(content, payload), "file content mismatch")
	require(read.GetEof(), "file read did not reach EOF")

	response, err = client.SandboxesAPI.DeleteSandbox(ownedContext, sandboxID).Execute()
	require(err == nil, fmt.Sprintf("delete failed: %v", err))
	require(response.StatusCode == 204, fmt.Sprintf("delete returned %d", response.StatusCode))
	deleted = true
	_, response, err = client.SandboxesAPI.GetSandbox(ownedContext, sandboxID).Execute()
	require(err != nil && response != nil && response.StatusCode == 404, "deleted sandbox remained addressable")

	audit, err := os.ReadFile(auditPath)
	require(err == nil, fmt.Sprintf("read audit: %v", err))
	require(!strings.Contains(string(audit), token), "bearer credential reached audit log")
	require(strings.Contains(string(audit), `"operation":"delete"`), "delete audit is absent")

	result := evidence{SchemaVersion: 1, Language: "go", SandboxID: sandboxID, Checks: checks}
	encoded, err := json.MarshalIndent(result, "", "  ")
	require(err == nil, fmt.Sprintf("encode evidence: %v", err))
	encoded = append(encoded, '\n')
	require(os.WriteFile(evidencePath, encoded, 0o644) == nil, "write evidence failed")
	fmt.Print(string(encoded))
}
