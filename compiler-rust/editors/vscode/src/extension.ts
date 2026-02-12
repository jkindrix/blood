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

    outputChannel.appendLine('Blood extension activated');
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
        // Design decision: Macro expansion requires LSP custom request protocol extension.
        // This will be implemented when the LSP server supports blood/expandMacro.
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
        // Design decision: Effect display requires LSP custom request protocol extension.
        // This will be implemented when the LSP server supports blood/showEffects.
        vscode.window.showInformationMessage('Effect display not yet implemented');
    }
}
