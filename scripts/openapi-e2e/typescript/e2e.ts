import { readFile, writeFile } from 'node:fs/promises';
import {
    CommandsApi,
    Configuration,
    FilesApi,
    ResponseError,
    SandboxState,
    SandboxesApi,
} from '@nya-a-cat/ferrobox';

const checks = [
    'generated-model-create',
    'bearer-auth-inspect',
    'typed-command-execution',
    'lossless-base64-output',
    'typed-file-roundtrip',
    'delete-and-stale-handle-rejection',
    'credential-redaction',
];

function requiredEnvironment(name: string): string {
    const value = process.env[name];
    if (!value) {
        throw new Error(`${name} is required`);
    }
    return value;
}

async function main(): Promise<void> {
    const apiUrl = requiredEnvironment('FERROBOX_API_URL');
    const auditPath = requiredEnvironment('FERROBOX_AUDIT_LOG');
    const evidencePath = requiredEnvironment('FERROBOX_OPENAPI_SDK_EVIDENCE');

    const publicSandboxes = new SandboxesApi(new Configuration({ basePath: apiUrl }));
    const created = await publicSandboxes.createSandbox({
        createSandboxRequest: {
            template: 'python',
            cpuCount: 1,
            memoryMb: 512,
            timeoutSeconds: 120,
            network: { internetAccess: false },
        },
    });
    if (created.state !== SandboxState.Running || !created.token) {
        throw new Error('created sandbox identity is invalid');
    }

    const configuration = new Configuration({ basePath: apiUrl, accessToken: created.token });
    const sandboxes = new SandboxesApi(configuration);
    const commands = new CommandsApi(configuration);
    const files = new FilesApi(configuration);
    let deleted = false;
    try {
        const inspected = await sandboxes.getSandbox({ id: created.sandboxId });
        if (inspected.sandboxId !== created.sandboxId || inspected.state !== SandboxState.Running) {
            throw new Error('inspect returned invalid sandbox state');
        }

        const executed = await commands.executeCommand({
            id: created.sandboxId,
            executeCommandRequest: {
                argv: ['python3', '-c', 'print(40 + 2)'],
                cwd: '/home/sandbox',
                environment: {},
                timeoutSeconds: 30,
                maxOutputBytes: 1048576,
            },
        });
        if (executed.stdout !== '42\n' || Buffer.from(executed.stdoutBase64, 'base64').toString('utf8') !== '42\n') {
            throw new Error('command output mismatch');
        }

        const payload = Buffer.from('generated-openapi-client\n', 'utf8');
        const written = await files.writeFile({
            id: created.sandboxId,
            writeFileRequest: {
                path: '/home/sandbox/openapi.txt',
                contentBase64: payload.toString('base64'),
                overwrite: false,
            },
        });
        if (written.bytesWritten !== payload.byteLength) {
            throw new Error('written byte count mismatch');
        }

        const read = await files.readFile({
            id: created.sandboxId,
            path: '/home/sandbox/openapi.txt',
            offset: 0,
            maxBytes: 1048576,
        });
        if (!Buffer.from(read.contentBase64, 'base64').equals(payload) || !read.eof) {
            throw new Error('file roundtrip mismatch');
        }

        await sandboxes.deleteSandbox({ id: created.sandboxId });
        deleted = true;
        try {
            await sandboxes.getSandbox({ id: created.sandboxId });
            throw new Error('deleted sandbox remained addressable');
        } catch (error) {
            if (!(error instanceof ResponseError) || error.response.status !== 404) {
                throw error;
            }
        }

        const audit = await readFile(auditPath, 'utf8');
        if (audit.includes(created.token) || !audit.includes('"operation":"delete"')) {
            throw new Error('audit credential-redaction check failed');
        }

        const evidence = {
            schema_version: 1,
            language: 'typescript',
            sandbox_id: created.sandboxId,
            checks,
        };
        const serialized = `${JSON.stringify(evidence, undefined, 2)}\n`;
        await writeFile(evidencePath, serialized, 'utf8');
        process.stdout.write(serialized);
    } finally {
        if (!deleted) {
            await sandboxes.deleteSandbox({ id: created.sandboxId }).catch(() => undefined);
        }
    }
}

void main().catch((error: unknown) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
});
