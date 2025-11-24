import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import { exec } from 'node:child_process';
import { promisify } from 'node:util';
import {
    LanguageClient,
    TransportKind,
    Trace
} from 'vscode-languageclient/node';
import type {
    LanguageClientOptions,
    ServerOptions,
} from 'vscode-languageclient/node';

const execAsync = promisify(exec);

let client: LanguageClient | undefined;
let outputChannel: vscode.OutputChannel;
let traceOutputChannel: vscode.OutputChannel;
let extensionContext: vscode.ExtensionContext | undefined;
let statusBarItem: vscode.StatusBarItem;

function findServerPath(context: vscode.ExtensionContext): string {
    const config = vscode.workspace.getConfiguration('otterlang');
    let serverPath = config.get<string>('lsp.serverPath', '');

    if (!serverPath || serverPath === 'otterlang-lsp') {
        const possiblePaths = [
            path.join(process.env.HOME || '', 'Downloads', 'otterlang', 'target', 'release', 'otterlang-lsp'),
            path.join(process.env.HOME || '', 'Downloads', 'otterlang', 'target', 'debug', 'otterlang-lsp'),
            path.join(context.extensionPath, '..', 'target', 'release', 'otterlang-lsp'),
            path.join(context.extensionPath, '..', 'target', 'debug', 'otterlang-lsp'),
            'otterlang-lsp'
        ];

        for (const possiblePath of possiblePaths) {
            try {
                if (fs.existsSync(possiblePath)) {
                    serverPath = possiblePath;
                    break;
                }
            } catch {
            }
        }

        if (!serverPath || serverPath === 'otterlang-lsp') {
            vscode.window.showErrorMessage(
                'OtterLang LSP server not found. Please set "otterlang.lsp.serverPath" in settings.'
            );
            serverPath = 'otterlang-lsp';
        }
    }

    return serverPath;
}

function createClient(context: vscode.ExtensionContext): LanguageClient {
    const serverPath = findServerPath(context);
    const config = vscode.workspace.getConfiguration('otterlang');
    const traceLevel = config.get<string>('lsp.trace', 'off');

    const serverOptions: ServerOptions = {
        run: { command: serverPath, transport: TransportKind.stdio },
        debug: { command: serverPath, transport: TransportKind.stdio }
    };

    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'otterlang' }],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher('**/*.ot')
        },
        outputChannel: outputChannel,
        traceOutputChannel: traceOutputChannel
    };

    const languageClient = new LanguageClient(
        'otterlang',
        'OtterLang Language Server',
        serverOptions,
        clientOptions
    );

    const trace = traceLevel === 'off' ? Trace.Off : traceLevel === 'messages' ? Trace.Messages : Trace.Verbose;
    languageClient.setTrace(trace);

    return languageClient;
}

function updateStatusBar() {
    if (client) {
        statusBarItem.text = '$(otter) OtterLang';
        statusBarItem.tooltip = 'OtterLang LSP: Running\nClick for options';
        statusBarItem.backgroundColor = undefined;
    } else {
        statusBarItem.text = '$(otter) OtterLang (Stopped)';
        statusBarItem.tooltip = 'OtterLang LSP: Stopped\nClick to start';
        statusBarItem.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');
    }
}

async function showStatusBarMenu() {
    const items: vscode.QuickPickItem[] = [
        {
            label: '$(debug-restart) Restart Server',
            description: 'Restart the language server',
            detail: 'Stops and starts the LSP server'
        },
        {
            label: client ? '$(debug-stop) Stop Server' : '$(play) Start Server',
            description: client ? 'Stop the language server' : 'Start the language server',
            detail: client ? 'Stops the LSP server' : 'Starts the LSP server'
        },
        {
            label: '$(output) Toggle Logs',
            description: 'Toggle LSP trace logs',
            detail: 'Enable or disable verbose logging'
        },
        {
            label: '$(terminal) Show Output',
            description: 'Show extension output',
            detail: 'Opens the output panel'
        },
        {
            label: '$(run) Run Current File',
            description: 'Execute the current OtterLang file',
            detail: 'Runs the file using the OtterLang interpreter'
        },
        {
            label: '$(symbol-color) Format Document',
            description: 'Format the current document',
            detail: 'Auto-formats the code'
        }
    ];

    const selected = await vscode.window.showQuickPick(items, {
        placeHolder: 'OtterLang Commands',
        matchOnDescription: true,
        matchOnDetail: true
    });

    if (!selected || !extensionContext) {
        return;
    }

    if (selected.label.includes('Restart Server')) {
        await restartServer(extensionContext);
    } else if (selected.label.includes('Stop Server')) {
        await stopServer();
    } else if (selected.label.includes('Start Server')) {
        await startServer(extensionContext);
    } else if (selected.label.includes('Toggle Logs')) {
        await toggleLogs();
    } else if (selected.label.includes('Show Output')) {
        showOutput();
    } else if (selected.label.includes('Run Current File')) {
        await runCurrentFile();
    } else if (selected.label.includes('Format Document')) {
        await formatDocument();
    }
}

async function startServer(context: vscode.ExtensionContext): Promise<void> {
    if (client) {
        await client.start();
        vscode.window.showInformationMessage('🦦 OtterLang Language Server started');
    } else {
        client = createClient(context);
        await client.start();
        vscode.window.showInformationMessage('🦦 OtterLang Language Server started');
    }
    updateStatusBar();
}

async function stopServer(): Promise<void> {
    if (client) {
        await client.stop();
        client = undefined;
        vscode.window.showInformationMessage('🦦 OtterLang Language Server stopped');
    }
    updateStatusBar();
}

async function restartServer(context: vscode.ExtensionContext): Promise<void> {
    if (client) {
        await client.stop();
        client = undefined;
    }
    client = createClient(context);
    await client.start();
    vscode.window.showInformationMessage('🦦 OtterLang Language Server restarted');
    updateStatusBar();
}

async function toggleLogs(): Promise<void> {
    const config = vscode.workspace.getConfiguration('otterlang');
    const currentTrace = config.get<string>('lsp.trace', 'off');
    const newTrace = currentTrace === 'off' ? 'verbose' : 'off';
    await config.update('lsp.trace', newTrace, vscode.ConfigurationTarget.Global);
    vscode.window.showInformationMessage(`LSP logs ${newTrace === 'off' ? 'disabled' : 'enabled'}`);
    if (client && extensionContext) {
        await restartServer(extensionContext);
    }
}

function showOutput(): void {
    outputChannel.show();
}

async function runCurrentFile(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== 'otterlang') {
        vscode.window.showErrorMessage('No OtterLang file is currently open');
        return;
    }

    const filePath = editor.document.fileName;
    const config = vscode.workspace.getConfiguration('otterlang');
    const interpreterPath = config.get<string>('interpreterPath', 'otterlang');

    // Save the file first
    await editor.document.save();

    const terminal = vscode.window.createTerminal({
        name: 'OtterLang',
        iconPath: new vscode.ThemeIcon('otter')
    });

    terminal.show();
    terminal.sendText(`${interpreterPath} "${filePath}"`);
}

async function formatDocument(): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== 'otterlang') {
        vscode.window.showErrorMessage('No OtterLang file is currently open');
        return;
    }

    await vscode.commands.executeCommand('editor.action.formatDocument');
}

export function activate(context: vscode.ExtensionContext) {
    extensionContext = context;
    outputChannel = vscode.window.createOutputChannel('OtterLang');
    traceOutputChannel = vscode.window.createOutputChannel('OtterLang Trace');

    // Create status bar item
    statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
    statusBarItem.command = 'otterlang.showMenu';
    statusBarItem.show();
    updateStatusBar();

    client = createClient(context);
    client.start();

    context.subscriptions.push(
        vscode.commands.registerCommand('otterlang.showMenu', showStatusBarMenu),
        vscode.commands.registerCommand('otterlang.restartServer', () => restartServer(context)),
        vscode.commands.registerCommand('otterlang.startServer', () => startServer(context)),
        vscode.commands.registerCommand('otterlang.stopServer', () => stopServer()),
        vscode.commands.registerCommand('otterlang.toggleLogs', () => toggleLogs()),
        vscode.commands.registerCommand('otterlang.showOutput', () => showOutput()),
        vscode.commands.registerCommand('otterlang.runFile', () => runCurrentFile()),
        vscode.commands.registerCommand('otterlang.formatDocument', () => formatDocument()),
        statusBarItem,
        outputChannel,
        traceOutputChannel
    );
}

export function deactivate(): Thenable<void> | undefined {
    if (!client) {
        return undefined;
    }
    return client.stop();
}
