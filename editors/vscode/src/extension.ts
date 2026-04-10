// Blood Language extension for VS Code
//
// This extension provides language support for the Blood programming language:
// - Syntax highlighting (via TextMate grammar)
// - Code completion, hover, go-to-definition (via LSP)
// - Formatting (via blood-fmt)
// - Diagnostics (via blood check)
// - Code snippets

import * as path from 'path';
import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;
let outputChannel: vscode.OutputChannel;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
    outputChannel = vscode.window.createOutputChannel('Blood');
    outputChannel.appendLine('Blood extension activating...');

    // Register commands
    context.subscriptions.push(
        vscode.commands.registerCommand('blood.restartServer', restartServer),
        vscode.commands.registerCommand('blood.runFile', runFile),
        vscode.commands.registerCommand('blood.checkFile', checkFile),
        vscode.commands.registerCommand('blood.formatFile', formatFile),
        vscode.commands.registerCommand('blood.showOutput', () => outputChannel.show()),
        vscode.commands.registerCommand('blood.openDocs', openDocs),
        vscode.commands.registerCommand('blood.expandMacro', expandMacro),
        vscode.commands.registerCommand('blood.showEffects', showEffects),
    );

    // Start the language server
    const config = vscode.workspace.getConfiguration('blood');
    if (config.get<boolean>('lsp.enable', true)) {
        await startLanguageServer(context);
    }

    // Register format provider
    if (config.get<boolean>('format.enable', true)) {
        context.subscriptions.push(
            vscode.languages.registerDocumentFormattingEditProvider('blood', {
                provideDocumentFormattingEdits,
            })
        );
    }

    // Set up format on save
    if (config.get<boolean>('format.onSave', true)) {
        context.subscriptions.push(
            vscode.workspace.onWillSaveTextDocument(async (event) => {
                if (event.document.languageId === 'blood') {
                    const edits = await provideDocumentFormattingEdits(event.document);
                    if (edits && edits.length > 0) {
                        event.waitUntil(Promise.resolve(edits));
                    }
                }
            })
        );
    }

    // Set up check on save (inline diagnostics without LSP)
    const diagnostics = vscode.languages.createDiagnosticCollection('blood');
    context.subscriptions.push(diagnostics);

    if (config.get<boolean>('checkOnSave', true)) {
        context.subscriptions.push(
            vscode.workspace.onDidSaveTextDocument(async (document) => {
                if (document.languageId === 'blood') {
                    await checkAndReport(document, config, diagnostics);
                }
            })
        );
        // Also check on open
        context.subscriptions.push(
            vscode.workspace.onDidOpenTextDocument(async (document) => {
                if (document.languageId === 'blood') {
                    await checkAndReport(document, config, diagnostics);
                }
            })
        );
    }

    outputChannel.appendLine('Blood extension activated');
}

/// Runs blood check on a document and reports diagnostics inline.
async function checkAndReport(
    document: vscode.TextDocument,
    config: vscode.WorkspaceConfiguration,
    diagnostics: vscode.DiagnosticCollection
): Promise<void> {
    const bloodPath = config.get<string>('path', 'blood');
    const filePath = document.fileName;
    const { spawn } = require('child_process');

    return new Promise<void>((resolve) => {
        const proc = spawn(bloodPath, ['check', filePath]);
        let stderr = '';

        proc.stderr.on('data', (data: Buffer) => {
            stderr += data.toString();
        });

        proc.on('close', (code: number) => {
            const diags: vscode.Diagnostic[] = [];

            if (code !== 0) {
                // Parse error output: "error[E0201]: message\n  --> file:line:col"
                const errorPattern = /error\[([^\]]+)\]: (.+)\n\s+-->\s+.+:(\d+):(\d+)/g;
                let match;
                while ((match = errorPattern.exec(stderr)) !== null) {
                    const message = match[2];
                    const line = parseInt(match[3], 10) - 1;
                    const col = parseInt(match[4], 10) - 1;

                    // Collect notes
                    const notePattern = /\s+=\s+note:\s+(.+)/g;
                    const remaining = stderr.substring(match.index + match[0].length);
                    let noteMatch;
                    let fullMessage = message;
                    while ((noteMatch = notePattern.exec(remaining)) !== null) {
                        fullMessage += '\n' + noteMatch[1];
                        if (noteMatch.index > 200) break; // don't scan too far
                    }

                    const range = new vscode.Range(line, col, line, col + 1);
                    const diag = new vscode.Diagnostic(range, fullMessage, vscode.DiagnosticSeverity.Error);
                    diag.code = match[1];
                    diag.source = 'blood';
                    diags.push(diag);
                }
            }

            diagnostics.set(document.uri, diags);
            resolve();
        });
    });
}

export async function deactivate(): Promise<void> {
    if (client) {
        await client.stop();
        client = undefined;
    }
}

async function startLanguageServer(context: vscode.ExtensionContext): Promise<void> {
    const config = vscode.workspace.getConfiguration('blood');
    const serverPath = config.get<string>('lsp.path', 'blood-lsp');

    // Server options
    const serverOptions: ServerOptions = {
        run: {
            command: serverPath,
            transport: TransportKind.stdio,
        },
        debug: {
            command: serverPath,
            transport: TransportKind.stdio,
        },
    };

    // Client options
    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'blood' }],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher('**/*.blood'),
        },
        outputChannel,
        traceOutputChannel: outputChannel,
    };

    // Create and start the client
    client = new LanguageClient(
        'blood',
        'Blood Language Server',
        serverOptions,
        clientOptions
    );

    try {
        await client.start();
        outputChannel.appendLine('Blood Language Server started');
    } catch (error) {
        outputChannel.appendLine(`Failed to start Blood Language Server: ${error}`);
        vscode.window.showWarningMessage(
            `Failed to start Blood Language Server. Make sure '${serverPath}' is installed and in your PATH.`
        );
    }
}

async function restartServer(): Promise<void> {
    outputChannel.appendLine('Restarting Blood Language Server...');

    if (client) {
        await client.stop();
        client = undefined;
    }

    const context = vscode.extensions.getExtension('blood-lang.blood-lang')?.exports;
    if (context) {
        await startLanguageServer(context);
    }
}

async function runFile(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== 'blood') {
        vscode.window.showErrorMessage('No Blood file is open');
        return;
    }

    // Save the file first
    await editor.document.save();

    const config = vscode.workspace.getConfiguration('blood');
    const bloodPath = config.get<string>('path', 'blood');
    const filePath = editor.document.fileName;

    const terminal = vscode.window.createTerminal('Blood Run');
    terminal.show();
    terminal.sendText(`${bloodPath} run "${filePath}"`);
}

async function checkFile(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== 'blood') {
        vscode.window.showErrorMessage('No Blood file is open');
        return;
    }

    // Save the file first
    await editor.document.save();

    const config = vscode.workspace.getConfiguration('blood');
    const bloodPath = config.get<string>('path', 'blood');
    const filePath = editor.document.fileName;

    outputChannel.appendLine(`Checking ${filePath}...`);

    const { spawn } = require('child_process');
    const process = spawn(bloodPath, ['check', filePath]);

    let stdout = '';
    let stderr = '';

    process.stdout.on('data', (data: Buffer) => {
        stdout += data.toString();
    });

    process.stderr.on('data', (data: Buffer) => {
        stderr += data.toString();
    });

    process.on('close', (code: number) => {
        if (code === 0) {
            outputChannel.appendLine('Check passed!');
            vscode.window.showInformationMessage('Blood: Check passed');
        } else {
            outputChannel.appendLine(`Check failed:\n${stderr}`);
            vscode.window.showErrorMessage('Blood: Check failed. See output for details.');
        }
    });
}

async function formatFile(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== 'blood') {
        vscode.window.showErrorMessage('No Blood file is open');
        return;
    }

    const edits = await provideDocumentFormattingEdits(editor.document);
    if (edits && edits.length > 0) {
        const edit = new vscode.WorkspaceEdit();
        edit.set(editor.document.uri, edits);
        await vscode.workspace.applyEdit(edit);
    }
}

async function provideDocumentFormattingEdits(
    document: vscode.TextDocument
): Promise<vscode.TextEdit[]> {
    const config = vscode.workspace.getConfiguration('blood');
    const fmtPath = config.get<string>('path', 'blood') + '-fmt';

    return new Promise((resolve, reject) => {
        const { spawn } = require('child_process');
        const process = spawn(fmtPath, ['--stdin']);

        let stdout = '';
        let stderr = '';

        process.stdout.on('data', (data: Buffer) => {
            stdout += data.toString();
        });

        process.stderr.on('data', (data: Buffer) => {
            stderr += data.toString();
        });

        process.on('close', (code: number) => {
            if (code === 0 && stdout) {
                const fullRange = new vscode.Range(
                    document.positionAt(0),
                    document.positionAt(document.getText().length)
                );
                resolve([vscode.TextEdit.replace(fullRange, stdout)]);
            } else {
                resolve([]);
            }
        });

        process.stdin.write(document.getText());
        process.stdin.end();
    });
}

async function openDocs(): Promise<void> {
    vscode.env.openExternal(
        vscode.Uri.parse('https://github.com/blood-lang/blood/tree/main/docs')
    );
}

async function expandMacro(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== 'blood') {
        vscode.window.showErrorMessage('No Blood file is open');
        return;
    }

    // Send expand macro request to LSP
    if (client) {
        const position = editor.selection.active;
        // TODO: Implement macro expansion via LSP custom request
        vscode.window.showInformationMessage('Macro expansion not yet implemented');
    }
}

async function showEffects(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== 'blood') {
        vscode.window.showErrorMessage('No Blood file is open');
        return;
    }

    // Send show effects request to LSP
    if (client) {
        const position = editor.selection.active;
        // TODO: Implement effect display via LSP custom request
        vscode.window.showInformationMessage('Effect display not yet implemented');
    }
}
