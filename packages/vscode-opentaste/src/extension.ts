// Extension lifecycle: find OpenTaste projects, feed editor events to the
// delta tracker, and hand corrected edits to `taste learn-delta`.
//
// Every failure is quiet. A missing binary, an uninitialised folder or a
// failed run is written to the "OpenTaste" output channel and nowhere else:
// a tool whose promise is staying out of the way never opens a modal.

import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';

import { learnDelta, resolveBinary, run } from './binaryRunner';
import { DeltaTracker, type ChangeReason, type ContentChange, type ReadLines } from './deltaTracker';
import { countEnabledRules } from './rules';
import { announce, StatusBar } from './statusBar';
import { isLearnable, parseTargets, type Targets } from './targets';

interface Project {
  root: string;
  binary: string;
  targets: Targets;
}

/** How often stale windows are dropped even if nothing is saved. */
const EXPIRE_EVERY_MS = 30_000;

export function activate(context: vscode.ExtensionContext): void {
  const log = vscode.window.createOutputChannel('OpenTaste');
  const statusBar = new StatusBar();
  const projects = new Map<string, Project>();

  const projectFor = (uri: vscode.Uri): Project | undefined => {
    if (uri.scheme !== 'file') {
      return undefined;
    }
    const folder = vscode.workspace.getWorkspaceFolder(uri);
    return folder ? projects.get(folder.uri.fsPath) : undefined;
  };
  const relativeTo = (project: Project, file: string) => path.relative(project.root, file).split(path.sep).join('/');

  const tracker = new DeltaTracker({
    isLearnable: (file) => {
      const project = projectFor(vscode.Uri.file(file));
      return project !== undefined && isLearnable(relativeTo(project, file), project.targets);
    },
  });

  const refreshStatus = () => {
    const active = vscode.window.activeTextEditor?.document.uri;
    const project = (active && projectFor(active)) ?? projects.values().next().value;
    if (!project) {
      statusBar.update(undefined);
      return;
    }
    let raw: string;
    try {
      raw = fs.readFileSync(path.join(project.root, '.taste', 'rules.json'), 'utf8');
    } catch {
      statusBar.update(undefined);
      return;
    }
    statusBar.update(countEnabledRules(raw));
  };

  const discover = async () => {
    projects.clear();
    const configured = vscode.workspace.getConfiguration('opentaste').get<string>('binaryPath', '');
    for (const folder of vscode.workspace.workspaceFolders ?? []) {
      const root = folder.uri.fsPath;
      if (!fs.existsSync(path.join(root, '.taste', 'rules.json'))) {
        continue;
      }
      const binary = resolveBinary(configured, root, {
        platform: process.platform,
        pathEnv: process.env.PATH ?? process.env.Path,
        pathExt: process.env.PATHEXT,
        exists: (file) => fs.existsSync(file),
      });
      if (!binary) {
        log.appendLine(`${root}: no \`taste\` binary found; set opentaste.binaryPath or install @opentaste/cli.`);
        continue;
      }
      const answer = await run(binary, ['targets', '--json'], { cwd: root });
      const targets = answer.code === 0 ? parseTargets(answer.stdout) : undefined;
      if (!targets) {
        log.appendLine(`${root}: \`${binary} targets --json\` failed; not learning here. ${answer.stderr.trim()}`);
        continue;
      }
      projects.set(root, { root, binary, targets });
      log.appendLine(`${root}: learning from edits with ${binary}`);
    }
    refreshStatus();
  };

  const readLinesOf =
    (document: vscode.TextDocument): ReadLines =>
    (start, end) => {
      const last = Math.min(end, document.lineCount - 1);
      if (last < start) {
        return '';
      }
      return document.getText(new vscode.Range(start, 0, last, document.lineAt(last).text.length));
    };

  const reasonOf = (reason: vscode.TextDocumentChangeReason | undefined): ChangeReason => {
    if (reason === vscode.TextDocumentChangeReason.Undo) {
      return 'undo';
    }
    if (reason === vscode.TextDocumentChangeReason.Redo) {
      return 'redo';
    }
    return undefined;
  };

  const learningEnabled = () => vscode.workspace.getConfiguration('opentaste').get<boolean>('learnFromEdits', true);

  const disable = async (project: Project, id: string) => {
    const result = await run(project.binary, ['disable', id, '--quiet'], { cwd: project.root });
    if (result.code !== 0) {
      log.appendLine(`could not disable ${id}: ${result.stderr.trim()}`);
    }
    refreshStatus();
  };

  context.subscriptions.push(
    log,
    statusBar,
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (!learningEnabled() || event.contentChanges.length === 0) {
        return;
      }
      const changes: ContentChange[] = event.contentChanges.map((change) => ({
        start: { line: change.range.start.line, character: change.range.start.character },
        end: { line: change.range.end.line, character: change.range.end.character },
        text: change.text,
      }));
      tracker.onChange(event.document.uri.fsPath, changes, reasonOf(event.reason), readLinesOf(event.document));
    }),
    // `onDidSave`, not `onWillSave`: only it sees the text that was saved.
    vscode.workspace.onDidSaveTextDocument(async (document) => {
      const project = projectFor(document.uri);
      if (!project) {
        return;
      }
      const deltas = tracker.onSave(document.uri.fsPath, readLinesOf(document));
      for (const delta of deltas) {
        const relative = relativeTo(project, delta.filePath);
        const learned = await learnDelta(project.binary, project.root, relative, delta.before, delta.after);
        if (!learned) {
          log.appendLine(`learn-delta for ${relative} failed or returned nothing usable`);
          continue;
        }
        for (const candidate of learned.candidates) {
          log.appendLine(`candidate ${candidate.id} (${Math.round(candidate.confidence * 100)}%): ${candidate.instruction}`);
        }
        refreshStatus();
        for (const rule of learned.promoted) {
          void announce(rule, (id) => disable(project, id));
        }
      }
    }),
    vscode.workspace.onDidCloseTextDocument((document) => tracker.forget(document.uri.fsPath)),
    vscode.window.onDidChangeActiveTextEditor(() => refreshStatus()),
    vscode.workspace.onDidChangeWorkspaceFolders(() => void discover()),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration('opentaste.binaryPath')) {
        void discover();
      }
    }),
    (() => {
      const watcher = vscode.workspace.createFileSystemWatcher('**/.taste/rules.json');
      watcher.onDidChange(refreshStatus);
      watcher.onDidCreate(() => void discover());
      watcher.onDidDelete(() => void discover());
      return watcher;
    })(),
    (() => {
      const timer = setInterval(() => tracker.expire(), EXPIRE_EVERY_MS);
      return { dispose: () => clearInterval(timer) };
    })(),
    vscode.commands.registerCommand('opentaste.openDashboard', () => {
      const active = vscode.window.activeTextEditor?.document.uri;
      const project = (active && projectFor(active)) ?? projects.values().next().value;
      if (!project) {
        void vscode.window.showInformationMessage('OpenTaste is not set up here. Run `taste init` in your project first.');
        return;
      }
      const terminal = vscode.window.createTerminal({ name: 'OpenTaste', cwd: project.root });
      terminal.sendText(`"${project.binary}" ui`);
      terminal.show();
    }),
    vscode.commands.registerCommand('opentaste.sync', async () => {
      for (const project of projects.values()) {
        const result = await run(project.binary, ['sync', '--quiet'], { cwd: project.root });
        if (result.code !== 0) {
          log.appendLine(`sync failed in ${project.root}: ${result.stderr.trim()}`);
        }
      }
      vscode.window.setStatusBarMessage('OpenTaste: rules synced to editor files', 3000);
    }),
  );

  void discover();
}

export function deactivate(): void {
  // Everything is disposed through context.subscriptions.
}
