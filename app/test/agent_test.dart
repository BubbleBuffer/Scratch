// The Agent section on the real core and the real agent bridge, with
// BubblePaw's fake Claude Code extension as the only model provider (it
// answers "ok", streamed as one paragraph). Never calls a real AI.
// - chats: create, rename and delete (delete asks first);
// - send: the reply streams in and stays;
// - a `js` cell (run as a script) is a collapsed row that opens;
// - an approval card decides (fake state), while the shipped bridge
//   runs Confirm tools without asking in Bypass;
// - while a run is going, Send says Steer and Stop cancels it;
// - turning the agent off in Settings > AI hides the sidebar row;
// - an AI task with the Agent runner ends done;
// - closing the agent ends its event stream at once.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/debounce.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/ui/ui.dart' show TextBtn;
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/ai/ai_store.dart';
import 'package:notes_app/src/rust/api/ai.dart';
import 'package:notes_app/src/rust/api/agent.dart'
    as bridge
    show AgentPermissionMode;

import 'ai_test.dart' show aiStore, closeAi;
import 'harness.dart';
import 'sync_helpers.dart';

/// BubblePaw's fake extension: NOTES_FAKE_EXTENSION, or its debug build
/// next to this repo.
String fakeExtension() {
  final forced = Platform.environment['NOTES_FAKE_EXTENSION'];
  if (forced != null && forced.isNotEmpty) return forced;
  final repo = File(bridgeLibrary()).parent.parent.parent;
  final sep = Platform.pathSeparator;
  final p =
      '${repo.parent.path}${sep}BubblePaw-base-agent${sep}target${sep}debug$sep'
      'bubblepaw-fake-extension${Platform.isWindows ? '.exe' : ''}';
  if (!File(p).existsSync()) {
    fail(
      'build BubblePaw\'s fake extension (cargo build -p bubblepaw-fake-extension) or set NOTES_FAKE_EXTENSION ($p)',
    );
  }
  return p;
}

/// Agent settings with the fake extension as Claude Code.
AiSettings fakeAgentSettings() {
  final dir = Directory(
    '${current.dir.path}${Platform.pathSeparator}claude-code',
  )..createSync(recursive: true);
  return AiSettings()
    ..agentEnabled = true
    ..agentModel = 'claude-code/code-model'
    ..agentBunPath = fakeExtension()
    ..agentClaudeCodeDir = dir.path;
}

/// Gives the test vault's agent the fake settings and opens it.
Future<AgentStore> openAgentStore(
  WidgetTester tester, {
  AiSettings? settings,
}) async {
  final store = AgentStore.attach(
    current.vault,
    settings ?? fakeAgentSettings(),
  );
  await real(tester, store.ensureOpen);
  expect(store.openError, isNull);
  await settle(tester);
  return store;
}

/// Opens the Agent section from the sidebar.
Future<void> openAgentScreen(WidgetTester tester) async {
  await tapAndSettle(tester, key('sidebar-agent'));
  expect(key('agent-screen'), findsOneWidget);
}

/// Pumps (real time passing) until [done], without waiting for bridge
/// calls to finish (a script may be running).
Future<void> pumpUntil(
  WidgetTester tester,
  bool Function() done, {
  Duration timeout = const Duration(seconds: 10),
  String Function()? diagnostics,
}) async {
  final sw = Stopwatch()..start();
  while (!done() && sw.elapsed < timeout) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 40)),
    );
    await tester.pump(const Duration(milliseconds: 40));
  }
  expect(
    done(),
    isTrue,
    reason: 'condition not reached in $timeout; ${diagnostics?.call() ?? ''}',
  );
}

Future<TestGesture> hover(WidgetTester tester, Finder target) async {
  final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
  await g.addPointer(location: Offset.zero);
  await g.moveTo(tester.getCenter(target));
  await settle(tester);
  return g;
}

String buttonLabel(WidgetTester tester, String k) => tester
    .widget<Text>(
      find.descendant(of: key(k), matching: find.byType(Text)).first,
    )
    .data!;

// Leave a stable cancellation window under a loaded full-suite run. Stop
// aborts this cell; successful tests do not wait for its natural completion.
const busyScript = 'const t = Date.now(); while (Date.now() - t < 30000) {} 1';

/// The opened result of the session's last `js` cell.
Future<String> lastCellResult(WidgetTester tester, AgentStore store) async {
  final call = store.state!.messages.expand((m) => m.toolCalls).last;
  final run = store.state!.runs.last;
  if (key('agent-cell-${call.id}-toggle').evaluate().isEmpty) {
    await tapAndSettle(tester, key('agent-work-${run.id}-toggle'));
    await tapAndSettle(tester, key('agent-work-${run.id}-code'));
  }
  await tapAndSettle(tester, key('agent-cell-${call.id}-toggle'));
  return textOf(tester, 'agent-cell-${call.id}-result');
}

void main() {
  setUpAll(initCore);

  appTest('chats: create, rename and delete', (tester) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await openAgentScreen(tester);
    expect(key('agent-no-chats'), findsOneWidget);
    expect(key('agent-session-project'), findsNothing);
    expect(key('agent-session-permission'), findsNothing);

    await tapAndSettle(tester, key('agent-new-chat'));
    await settleUntil(tester, () => store.sessions.length == 1);
    final id = store.sessions.single.id;
    expect(store.openId, id);
    expect(
      find.descendant(
        of: key('agent-session-$id'),
        matching: find.text('New chat'),
      ),
      findsOneWidget,
    );
    expect(key('agent-empty'), findsOneWidget);

    // Rename from the row's menu.
    final g = await hover(tester, key('agent-session-$id'));
    await tapAndSettle(tester, key('agent-session-menu-$id'));
    await g.removePointer();
    await tapAndSettle(tester, key('agent-rename'));
    await enterField(tester, 'agent-rename-field', 'Garden questions');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await settleUntil(
      tester,
      () => store.sessions.single.title == 'Garden questions',
    );
    expect(
      find.descendant(
        of: key('agent-session-$id'),
        matching: find.text('Garden questions'),
      ),
      findsOneWidget,
    );

    // A second chat lists first; the filter narrows by title.
    await tapAndSettle(tester, key('agent-new-chat'));
    await settleUntil(tester, () => store.sessions.length == 2);
    expect(store.sessions.first.title, 'New chat');
    await enterField(tester, 'agent-filter', 'garden');
    expect(store.visibleSessions.map((s) => s.id), [id]);
    expect(key('agent-session-${store.sessions.first.id}'), findsNothing);
    await enterField(tester, 'agent-filter', '');

    // Delete asks first.
    final g2 = await hover(tester, key('agent-session-$id'));
    await tapAndSettle(tester, key('agent-session-menu-$id'));
    await g2.removePointer();
    await tapAndSettle(tester, key('agent-delete'));
    expect(find.text('Delete this chat?'), findsOneWidget);
    await tapAndSettle(tester, find.text('Delete').last);
    await settleUntil(tester, () => store.sessions.length == 1);
    expect(store.sessions.any((s) => s.id == id), isFalse);
    expect(key('agent-session-$id'), findsNothing);
  });

  appTest('send: the reply streams in and stays', (tester) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await openAgentScreen(tester);
    expect(textOf(tester, 'agent-empty'), contains('Ask the agent'));
    // The model picker shows the default.
    expect(
      find.descendant(
        of: key('agent-model'),
        matching: find.text('Code model'),
      ),
      findsOneWidget,
    );

    // Shift+Enter is a new line; Enter sends and starts a chat named after
    // the first line.
    await tester.tap(key('agent-composer'));
    await settle(tester);
    await tester.enterText(key('agent-composer'), 'Say ok');
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await settle(tester);
    expect(store.sessions, isEmpty);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await settleUntil(
      tester,
      () =>
          (store.state?.messages.any(
                (m) => m.role == 'assistant' && m.content == 'ok',
              ) ??
              false) &&
          !store.running,
      timeout: const Duration(seconds: 30),
    );
    expect(store.sessions.single.title, 'Say ok');
    expect(store.paragraphsSeen, greaterThan(0));
    expect(store.streaming, isEmpty);
    expect(find.text('Say ok'), findsWidgets);
    expect(find.text('ok'), findsOneWidget);
    expect(key('agent-empty'), findsNothing);
    expect(buttonLabel(tester, 'agent-send'), 'Send');
  });

  appTest('a js cell is a collapsed row that opens', (tester) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await openAgentScreen(tester);
    final s = await real(tester, () => store.createSession(title: 'Script'));
    final run = await real(
      tester,
      () => store.runScript(s!.id, 'Count pages', 'Math.max(1,\n  3)'),
    );
    expect(run?.status, 'completed');
    await settle(tester);
    final call = store.state!.messages.expand((m) => m.toolCalls).single;
    final cell = 'agent-cell-${call.id}';
    expect(key(cell), findsNothing);
    await tapAndSettle(tester, key('agent-work-${run!.id}-toggle'));
    expect(key(cell), findsNothing);
    await tapAndSettle(tester, key('agent-work-${run.id}-code'));
    expect(key(cell), findsOneWidget);
    expect(
      find.descendant(of: key(cell), matching: find.text('Math.max(1,')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: key(cell), matching: find.text('Done')),
      findsOneWidget,
    );
    expect(key('$cell-code'), findsNothing);
    // The tool message itself is not shown on its own.
    final tool = store.state!.messages.firstWhere((m) => m.role == 'tool');
    expect(key('agent-msg-${tool.id}'), findsNothing);

    await tapAndSettle(tester, key('$cell-toggle'));
    expect(textOf(tester, '$cell-code'), 'Math.max(1,\n  3)');
    expect(textOf(tester, '$cell-result'), '3');
    await tapAndSettle(tester, key('$cell-toggle'));
    expect(key('$cell-code'), findsNothing);
  });

  appTest('an approval card decides (fake state)', (tester) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await openAgentScreen(tester);
    final s = (await real(
      tester,
      () => store.createSession(title: 'Approvals'),
    ))!;
    AgentApproval approval(String id) => AgentApproval(
      id: id,
      sessionId: s.id,
      runId: 'run',
      callId: 'call-$id',
      tool: 'delete_page',
      argumentsJson: '{"page":"Garden plan"}',
      state: 'pending',
      createdAt: '2026-10-06T11:20:00Z',
    );
    store.debugAddApproval(approval('a1'));
    await settle(tester);
    expect(textOf(tester, 'agent-approval-tool-a1'), 'delete_page');
    expect(
      textOf(tester, 'agent-approval-args-a1'),
      contains('"page": "Garden plan"'),
    );
    expect(key('agent-approve-a1'), findsOneWidget);
    expect(key('agent-approve-session-a1'), findsOneWidget);
    await tapAndSettle(tester, key('agent-approve-a1'));
    expect(key('agent-approval-a1'), findsNothing);

    // Deny opens an optional reason, then decides.
    store.debugAddApproval(approval('a2'));
    await settle(tester);
    await tapAndSettle(tester, key('agent-deny-a2'));
    expect(key('agent-deny-reason-a2'), findsOneWidget);
    expect(key('agent-approve-a2'), findsNothing);
    await enterField(tester, 'agent-deny-reason-a2', 'Keep it');
    await tapAndSettle(tester, key('agent-deny-confirm-a2'));
    expect(key('agent-approval-a2'), findsNothing);
  });

  appTest('Confirm tools run without approval in the shipped Bypass agent', (
    tester,
  ) async {
    AgentStore.testTools = true;
    try {
      await pumpApp(tester, mobile: false);
      final store = await openAgentStore(tester);
      await openAgentScreen(tester);
      final session = (await real(
        tester,
        () => store.createSession(title: 'Bypass'),
      ))!;
      final run = await real(
        tester,
        () => store.runScript(
          session.id,
          'Confirm tool',
          "return await tools.test_confirm({text: 'first'});",
        ),
      );
      expect(run!.status, 'completed');
      await settle(tester);
      expect(store.pendingApprovals, isEmpty);
      expect(await lastCellResult(tester, store), '"confirmed: first"');
    } finally {
      AgentStore.testTools = false;
    }
  });

  appTest(
    'attaching settings to a precreated store does not reopen a newly opened agent later',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      final precreated = AgentStore.of(tv.app);
      expect(AgentStore.existing(tv.vault), same(precreated));
      final wasInstant = Debounce.instant;
      Debounce.instant = false;
      try {
        // Schedule settings timers and the wait on the real clock together.
        // Pumping only fake time would not exercise the delayed reopen race.
        await tester.runAsync(() async {
          final store = AgentStore.attach(tv.vault, fakeAgentSettings());
          expect(store, same(precreated));
          final agent = await store.ensureOpen();
          expect(agent, isNotNull);
          final session = await store.createSession(
            title: 'Survives settings attachment',
          );
          expect(session, isNotNull);
          await Future<void>.delayed(const Duration(milliseconds: 850));
          expect(store.isOpen, isTrue);
          expect(await store.ensureOpen(), same(agent));
          expect(store.openId, session!.id);
          expect(store.state?.session.id, session.id);
        });
      } finally {
        Debounce.instant = wasInstant;
      }
      await settle(tester);
    },
  );

  appTest('closing the agent ends its event stream at once', (tester) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    expect(store.isOpen, isTrue);
    final watch = Stopwatch()..start();
    await real(tester, store.close);
    // Without the close signal the cancel would wait for another event
    // until the guard timeout.
    expect(watch.elapsed, lessThan(const Duration(seconds: 1)));
    expect(AgentStore.closeTimeout, greaterThan(const Duration(seconds: 1)));
  });

  appTest('while a run is going, Send steers and Stop cancels', (tester) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await openAgentScreen(tester);
    final s = (await real(tester, () => store.createSession(title: 'Busy')))!;
    expect(key('agent-stop'), findsNothing);
    expect(buttonLabel(tester, 'agent-send'), 'Send');

    final done = Completer<void>();
    await tester.runAsync(() async {
      unawaited(
        store.runScript(s.id, 'Busy', busyScript).whenComplete(done.complete),
      );
    });
    await pumpUntil(
      tester,
      () => key('agent-stop').evaluate().isNotEmpty,
      timeout: const Duration(seconds: 30),
      diagnostics: () =>
          'script completed=${done.isCompleted}; runs=${store.state?.runs.map((r) => r.status).join(',')}; error=${store.error}',
    );
    expect(buttonLabel(tester, 'agent-send'), 'Steer');
    expect(find.text('Steer the agent…'), findsOneWidget);
    expect(key('agent-working'), findsOneWidget);
    expect(find.byType(CodeCellRow), findsNothing);

    await tester.tap(key('agent-stop'));
    await pumpUntil(
      tester,
      () => done.isCompleted,
      timeout: const Duration(seconds: 15),
    );
    await settleUntil(tester, () => !store.running);
    expect(key('agent-stop'), findsNothing);
    expect(buttonLabel(tester, 'agent-send'), 'Send');
    final run = store.state!.runs.single;
    expect(run.status, 'cancelled');
    expect(find.text('Stopped.'), findsOneWidget);
  });

  appTest('turning the agent off in Settings > AI hides its sidebar row', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    expect(key('sidebar-agent'), findsOneWidget);
    await tapAndSettle(tester, key('sidebar-settings'));
    await tapAndSettle(tester, key('settings-nav-ai'));
    expect(key('agent-enabled-toggle'), findsOneWidget);
    await tapAndSettle(tester, key('agent-enabled-toggle'));
    await settleUntil(tester, () => !store.isOpen);
    expect(store.settings.agentEnabled, isFalse);
    expect(key('agent-default-model'), findsNothing);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(tester);
    expect(key('sidebar-agent'), findsNothing);
  });

  appTest('computer settings reopen the local agent and hide on Hub', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    final ai = await aiStore(tester, settings: store.settings);
    await settleUntil(tester, () => store.hostInfo != null);
    expect(store.hostInfo?.packs, contains('desktop'));
    expect(store.hostInfo?.tools, contains('exec'));
    await tapAndSettle(tester, key('sidebar-settings'));
    await tapAndSettle(tester, key('settings-nav-ai'));
    expect(key('agent-desktop-tools'), findsOneWidget);
    expect(key('agent-permissions'), findsNothing);
    expect(key('agent-projects'), findsOneWidget);
    expect(key('agent-desktop-root'), findsNothing);
    expect(
      textOf(tester, 'agent-host-info'),
      'Runs on: This computer · tools: vault, files & shell',
    );

    final first = await real(tester, store.ensureOpen);
    store.settings.update((s) => s.desktopTools = false);
    await settleUntil(
      tester,
      () =>
          store.hostInfo != null && !store.hostInfo!.packs.contains('desktop'),
    );
    final withoutFiles = await real(tester, store.ensureOpen);
    expect(identical(first, withoutFiles), isFalse);
    expect(store.hostInfo!.tools, isNot(contains('exec')));

    store.settings.update((s) {
      s.desktopTools = true;
      s.desktopRoot = tv.dir.path;
    });
    await settleUntil(
      tester,
      () => store.hostInfo?.packs.contains('desktop') ?? false,
    );
    final rooted = await real(tester, store.ensureOpen);
    expect(identical(withoutFiles, rooted), isFalse);

    final otherRoot = Directory(
      '${tv.dir.path}${Platform.pathSeparator}computer-files',
    )..createSync();
    store.settings.update((s) => s.desktopRoot = otherRoot.path);
    await settleUntil(tester, () => store.hostInfo != null);
    final changedRoot = await real(tester, store.ensureOpen);
    expect(
      identical(rooted, changedRoot),
      isFalse,
      reason: 'changing just the root reopens too',
    );

    // Render the selected Hub state without opening an external connection.
    store.settings.agentHost = AgentHost.hub;
    tv.app.settings
      ..clientUrl = 'https://notes.breadcat.cc'
      ..clientToken = 'test-token';
    store.debugSetHostInfo(
      const AgentHostInfo(
        kind: 'hub',
        permissionMode: bridge.AgentPermissionMode.auto,
        projects: false,
        packs: [],
        tools: ['search_notes'],
        providers: ['codex'],
        canRunTasks: false,
        version: 'test',
      ),
    );
    await tester.pump();
    expect(key('agent-desktop-tools'), findsNothing);
    expect(key('agent-desktop-root'), findsNothing);
    expect(key('agent-permissions'), findsNothing);
    expect(key('agent-projects'), findsNothing);
    expect(
      textOf(tester, 'agent-host-info'),
      'Runs on: notes.breadcat.cc · tools: vault',
    );
    await closeAi(tester, ai);
  });

  appTest('the project chip sets the session without reopening', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    final original = await real(tester, store.ensureOpen);
    final folder = Directory(
      '${tv.dir.path}${Platform.pathSeparator}second-project',
    )..createSync();
    await real(tester, () => store.addProject('Second', folder.path));
    expect(store.projects.map((p) => p.name), contains('Second'));
    await openAgentScreen(tester);
    final session = (await real(
      tester,
      () => store.createSession(title: 'Projects'),
    ))!;
    await settle(tester);
    expect(key('agent-session-project'), findsOneWidget);
    await tapAndSettle(tester, key('agent-session-project'));
    await tapAndSettle(tester, key('option-Second'));
    await settleUntil(
      tester,
      () => store.state?.session.projectName == 'Second',
    );
    expect(store.state!.session.id, session.id);
    expect(
      store.sessions.firstWhere((s) => s.id == session.id).projectName,
      'Second',
    );

    expect(key('agent-session-permission'), findsNothing);
    expect(identical(original, await real(tester, store.ensureOpen)), isTrue);
    final project = store.projects.firstWhere((p) => p.name == 'Second');
    await real(tester, () => store.renameProject(project.id, 'Renamed'));
    await settle(tester);
    expect(store.state!.session.projectName, 'Renamed');
    await real(tester, () => store.removeProject(project.id));
    expect(store.projects, hasLength(1));
    expect(store.state!.session.projectId, store.projects.single.id);
  });

  appTest('projects settings block last removal and hide permission controls', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    final ai = await aiStore(tester, settings: store.settings);
    try {
      await tapAndSettle(tester, key('sidebar-settings'));
      await tapAndSettle(tester, key('settings-nav-ai'));
      expect(key('agent-permissions'), findsNothing);
      expect(find.text('Permissions'), findsNothing);
      expect(key('agent-projects'), findsOneWidget);
      final removal = tester.widget<TextBtn>(
        key('agent-project-remove-${store.projects.single.id}'),
      );
      expect(removal.onTap, isNull);
    } finally {
      await closeAi(tester, ai);
    }
  });

  appTest(
    'legacy Manual preference and mode file are preserved while the bridge uses Bypass',
    (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      final settingsFile = File(
        '${tv.dir.path}${Platform.pathSeparator}ai_settings.json',
      );
      final legacy = fakeAgentSettings().toJson()
        ..remove('agentPermissionMode')
        ..['agentApprovals'] = 'ask';
      final savedSettings = jsonEncode(legacy);
      settingsFile.writeAsStringSync(savedSettings);
      final settings = AiSettings.load(settingsFile);
      expect(settings.agentPermissionMode, AgentPermissionMode.manual);
      final modeFile = File(
        '${tv.dir.path}${Platform.pathSeparator}agent${Platform.pathSeparator}permission-mode.json',
      );
      modeFile.parent.createSync(recursive: true);
      const savedMode = '"manual"\n';
      modeFile.writeAsStringSync(savedMode);
      final store = AgentStore.attach(tv.vault, settings, dataDir: tv.dir.path);
      await real(tester, store.ensureOpen);
      await settle(tester);
      final agent = (await real(tester, store.ensureOpen))!;
      expect(store.permissionMode, AgentPermissionMode.bypass);
      expect(store.hostInfo!.permissionMode, bridge.AgentPermissionMode.bypass);
      expect(
        (await real(tester, agent.hostInfo)).permissionMode,
        bridge.AgentPermissionMode.bypass,
      );
      expect(settings.agentPermissionMode, AgentPermissionMode.manual);
      expect(settingsFile.readAsStringSync(), savedSettings);
      expect(modeFile.readAsStringSync(), savedMode);
      settings.update((s) => s.agentPermissionMode = AgentPermissionMode.auto);
      await settle(tester);
      expect(identical(agent, await real(tester, store.ensureOpen)), isTrue);
      expect(store.permissionMode, AgentPermissionMode.bypass);
      expect(modeFile.readAsStringSync(), savedMode);
    },
  );

  appTest('project_open approval describes the folder and project', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final store = await openAgentStore(tester);
    await openAgentScreen(tester);
    final session = (await real(
      tester,
      () => store.createSession(title: 'Open project'),
    ))!;
    store.debugAddApproval(
      AgentApproval(
        id: 'project-open',
        sessionId: session.id,
        runId: 'run',
        callId: 'call',
        tool: 'project_open',
        argumentsJson: '{"path":"C:/Work/Garden","name":"Garden"}',
        state: 'pending',
        createdAt: '2026-10-09T12:00:00Z',
      ),
    );
    await settle(tester);
    expect(
      textOf(tester, 'agent-approval-args-project-open'),
      'Open folder C:/Work/Garden as project Garden?',
    );
  });

  appTest('task availability follows host capabilities', (tester) async {
    await pumpApp(tester, mobile: false);
    final settings = fakeAgentSettings();
    final agent = await openAgentStore(tester, settings: settings);
    final ai = await aiStore(tester, settings: settings);
    try {
      await settleUntil(tester, () => ai.canAnswer);
      expect(ai.canAnswer, isTrue);
      agent.debugSetHostInfo(
        const AgentHostInfo(
          kind: 'desktop',
          permissionMode: bridge.AgentPermissionMode.auto,
          projects: true,
          packs: ['desktop'],
          tools: ['exec'],
          providers: ['codex'],
          canRunTasks: false,
          version: 'test',
        ),
      );
      expect(
        ai.canAnswer,
        isFalse,
        reason: 'a desktop kind does not grant task support',
      );
      agent.debugSetHostInfo(
        const AgentHostInfo(
          kind: 'hub',
          permissionMode: bridge.AgentPermissionMode.auto,
          projects: false,
          packs: [],
          tools: ['search_notes'],
          providers: ['codex'],
          canRunTasks: true,
          version: 'test',
        ),
      );
      expect(ai.canAnswer, isTrue, reason: 'task support comes from host_info');
      await settle(tester);
    } finally {
      await closeAi(tester, ai);
    }
  });

  appTest('an AI task with the Agent runner ends done', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final settings = fakeAgentSettings();
    expect(
      settings.runner,
      AiRunner.agent,
      reason: 'the default when the agent is on',
    );
    await openAgentStore(tester, settings: settings);
    final store = await aiStore(tester, settings: settings);
    try {
      await real(
        tester,
        () => aiCreateTask(
          vault: tv.vault.raw,
          prompt: 'Say ok',
          page: tv.pages['Garden plan']!,
          after: null,
          runner: '',
        ),
      );
      await settleUntil(
        tester,
        () =>
            store.tasks.isNotEmpty &&
            store.tasks.first.status != AiStatus.queued &&
            store.tasks.first.status != AiStatus.running,
        timeout: const Duration(seconds: 30),
      );
      expect(store.tasks.single.status, AiStatus.done);
    } finally {
      await closeAi(tester, store);
    }
  });
}
