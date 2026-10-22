// Ask AI from the selection toolbar, the block menu and sheet, and the
// page menu, on desktop and phone, and tasks assigned to the agent. Runs
// on the synced default runner:
// - this computer: the in-app agent with BubblePaw's fake Claude Code
//   extension (agent_test.dart), which answers "ok";
// - the Hub: a fake hub jobs client records the jobs (the hub side is
//   tested in crates/hub/tests/jobs.rs).
// Never calls a real AI.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/addons/tasks/api.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/ai/ai_store.dart';
import 'package:notes_app/features/ai/hub_jobs_client.dart';
import 'package:notes_app/registry/note_type_registry.dart';

import 'addons/tasks/task_fixture.dart' show pumpTaskHost;
import 'agent_test.dart' show fakeAgentSettings, openAgentStore;
import 'ai_test.dart' show aiStore, closeAi;
import 'editor_agent_partb_test.dart' show editor, open;
import 'harness.dart';
import 'sync_helpers.dart';

class FakeHubJobs implements HubJobs {
  final created = <({String prompt, String page, String? after})>[];
  final cancelled = <String>[];

  @override
  Future<HubJob> create({required String prompt, required String page, String? after}) async {
    created.add((prompt: prompt, page: page, after: after));
    return HubJob(id: 'job-${created.length}', status: 'queued', tools: 'everything');
  }

  @override
  Future<void> cancel(String id) async => cancelled.add(id);
}

/// Signs this device in to a (fake) hub and records its jobs.
FakeHubJobs useHub(WidgetTester tester, AiSettings settings) {
  final jobs = FakeHubJobs();
  final app = current.app.settings;
  AgentStore.attach(current.vault, settings).useAppSettings(app);
  app.update((s) => s
    ..clientUrl = 'https://hub.example.com'
    ..clientToken = 'test-token');
  AiStore.hubJobsOverride = (hub, device) {
    expect(hub.url, 'https://hub.example.com');
    expect(device, current.vault.deviceId);
    return jobs;
  };
  addTearDown(() {
    AiStore.hubJobsOverride = null;
    app
      ..clientUrl = null
      ..clientToken = null;
  });
  return jobs;
}

/// A phone's AI store (no local runner).
Future<AiStore> phoneAi(WidgetTester tester) async {
  final store = AiStore.attach(current.vault, current.app.feed, AiSettings()..agentEnabled = false);
  await real(tester, store.start);
  return store;
}

/// A desktop AI store whose agent (the fake extension) answers Ask AI here.
Future<AiStore> agentAi(WidgetTester tester) async {
  final settings = fakeAgentSettings();
  await openAgentStore(tester, settings: settings);
  return aiStore(tester, settings: settings);
}

List<String> textsOf(List<dynamic> blocks) => [for (final b in blocks) b.text as String];

Future<void> send(WidgetTester tester, String prompt) async {
  await enterField(tester, 'ask-ai-field', prompt);
  await tester.tap(key('ask-ai-send'));
  await settle(tester);
}

void main() {
  setUpAll(initCore);
  setUp(() => HttpOverrides.global = null);

  group('desktop', () {
    appTest("selection toolbar: this computer's agent answers it", (t) async {
      final tv = await pumpApp(t, mobile: false);
      final store = await agentAi(t);
      try {
        final id = tv.pages['Recipes']!;
        await open(t, id);
        final ed = editor(t, id);
        final n = ed.doc!.flat.firstWhere((n) => n.text.length >= 8);
        ed.focusBlock(n, 0);
        await settle(t);
        const selection = TextSelection(baseOffset: 2, extentOffset: 7);
        ed.ctl(n).selection = selection;
        ed.onSelectionChanged(n, selection, SelectionChangedCause.keyboard);
        await t.pump();
        await t.pump();
        await tapAndSettle(t, key('fmt-ask-ai'));
        expect(key('ask-ai-field'), findsOneWidget);
        expect(textOf(t, 'ask-ai-selection'), '“${n.text.substring(2, 7)}”');
        expect(textOf(t, 'ask-ai-note'), 'The agent answers on this device and writes into the page.');
        await send(t, 'Say ok');
        await settleUntil(t, () => store.tasks.isNotEmpty && store.tasks.first.status == AiStatus.done,
            timeout: const Duration(seconds: 30));
        final task = store.tasks.first;
        // Pinned to this device, about this page.
        expect(task.node, tv.vault.deviceId);
        expect(task.page, id);
        expect(task.prompt, contains('> ${n.text.substring(2, 7)}'));
        final texts = textsOf(await blocksOf(t, id));
        expect(texts.indexOf('ok'), greaterThan(texts.indexOf(n.text)), reason: '$texts');
      } finally {
        await closeAi(t, store);
      }
    });

    appTest('block menu: the answer goes after the block', (t) async {
      final tv = await pumpApp(t, mobile: false);
      final store = await agentAi(t);
      try {
        final id = tv.pages['Garden plan']!;
        await open(t, id);
        final ed = editor(t, id);
        final n = ed.doc!.flat.first;
        ed.openBlockMenu(n, const Rect.fromLTWH(400, 150, 18, 24));
        await t.pump();
        await tapAndSettle(t, key('menu-ask-ai'));
        expect(textOf(t, 'ask-ai-selection'), contains(n.text.substring(0, 8)));
        await send(t, 'List what to plant');
        await settleUntil(t, () => store.tasks.isNotEmpty && store.tasks.first.status == AiStatus.done,
            timeout: const Duration(seconds: 30));
        final texts = textsOf(await blocksOf(t, id));
        expect(texts.indexOf('ok'), greaterThan(texts.indexOf(n.text)), reason: '$texts');
      } finally {
        await closeAi(t, store);
      }
    });

    appTest('page menu with the Hub as default runner: a hub job about the page', (t) async {
      final tv = await pumpApp(t, mobile: false);
      final store = await aiStore(t);
      try {
        final jobs = useHub(t, store.settings);
        final agents = AgentStore.attach(tv.vault, store.settings);
        await real(t, () => agents.setVaultDefaultHost(AgentHost.hub));
        await settle(t);
        final id = tv.pages['Recipes']!;
        await open(t, id);
        await tapAndSettle(t, key('page-menu'));
        await tapAndSettle(t, key('page-menu-ask-ai'));
        expect(textOf(t, 'ask-ai-note'), 'The agent runs on your hub and writes into the page.');
        expect(key('ask-ai-selection'), findsNothing);
        await send(t, 'Summarize this page');
        expect(jobs.created, hasLength(1));
        expect(jobs.created.single.page, id);
        expect(jobs.created.single.after, isNull);
        expect(jobs.created.single.prompt, 'Summarize this page');
        expect(key('ask-ai-field'), findsNothing);
        // Nothing ran here.
        expect(store.tasks, isEmpty);
      } finally {
        await closeAi(t, store);
      }
    });
  });

  group('phone', () {
    appTest('block sheet: a hub job about the block (phones use the Hub)', (t) async {
      final tv = await pumpApp(t, mobile: true);
      final store = await phoneAi(t);
      try {
        final jobs = useHub(t, store.settings);
        final id = tv.pages['Recipes']!;
        await open(t, id);
        final ed = editor(t, id);
        final n = ed.doc!.flat.firstWhere((n) => n.text.length >= 8);
        await t.longPress(editableWith(n.text));
        await settle(t);
        await tapAndSettle(t, key('sheet-ask-ai'));
        expect(textOf(t, 'ask-ai-note'), contains('phones always use it'));
        await send(t, 'Explain this');
        expect(jobs.created, hasLength(1));
        expect(jobs.created.single.page, id);
        expect(jobs.created.single.after, n.id);
        expect(jobs.created.single.prompt, startsWith('Explain this'));
        expect(jobs.created.single.prompt, contains('> ${n.text.split('\n').first}'));
      } finally {
        await closeAi(t, store);
      }
    });

    appTest('page menu: a hub job; signed out, the entry stays and says to sign in', (t) async {
      final tv = await pumpApp(t, mobile: true);
      final store = await phoneAi(t);
      try {
        final id = tv.pages['Recipes']!;
        await open(t, id);
        // Signed out: still offered, and it says where the agent runs.
        await tapAndSettle(t, key('page-menu'));
        await tapAndSettle(t, key('page-menu-ask-ai'));
        expect(textOf(t, 'ask-ai-note'), contains('Sign in to your hub'));
        await enterField(t, 'ask-ai-field', 'Summarize');
        await t.tap(key('ask-ai-send'));
        await settle(t);
        expect(textOf(t, 'ask-ai-note'), contains('Sign in to your hub'));
        await t.tap(find.text('Cancel'));
        await settle(t);

        final jobs = useHub(t, store.settings);
        await tapAndSettle(t, key('page-menu'));
        await tapAndSettle(t, key('page-menu-ask-ai'));
        await send(t, 'Summarize');
        expect(jobs.created.single.page, id);
        expect(jobs.created.single.after, isNull);
      } finally {
        await closeAi(t, store);
      }
    });
  });

  group('assign to agent', () {
    Future<String> taskPage(WidgetTester t, String title) => core(t, (v) async {
          final page = await v.createPage(title: title, pageType: taskType);
          await saveTask(v, page, const TaskData(date: '2026-10-08'));
          return page;
        });

    Widget taskNote(String page, String title) => SizedBox(
          height: 700,
          child: TaskNote(
            args: NoteArgs(pageId: page, title: title, editor: const SizedBox.shrink()),
            presentation: TaskPresentation.page,
          ),
        );

    Future<void> assign(WidgetTester t, String from, String to) async {
      await tapAndSettle(t, find.text(from).last);
      await tapAndSettle(t, find.text(to).last);
      await settleUntil(t, () => false, timeout: const Duration(milliseconds: 400));
    }

    for (final mobile in [false, true]) {
      appTest('${mobile ? 'phone' : 'desktop'}: one hub job; reassigning starts none; unassigning cancels',
          (t) async {
        final tv = await pumpApp(t, mobile: mobile, empty: true);
        final store = mobile ? await phoneAi(t) : await aiStore(t);
        try {
          final jobs = useHub(t, store.settings);
          if (!mobile) {
            await real(t, () => AgentStore.attach(tv.vault, store.settings).setVaultDefaultHost(AgentHost.hub));
          }
          final page = await taskPage(t, 'Book the flights');
          await pumpTaskHost(t, tv, taskNote(page, 'Book the flights'), mobile: mobile);
          expect(key('task-agent-note'), findsNothing);
          expect(find.text('Scheduled runs come later.'), findsNothing);

          await assign(t, 'Me', 'Agent');
          expect(jobs.created, hasLength(1));
          expect(jobs.created.single.page, page);
          expect(jobs.created.single.prompt, contains('Book the flights'));
          final data = await core(t, (v) => loadTask(v, page));
          expect(data.assignee, 'agent');
          expect(data.agentRun, 'job-1');
          // Not synced here yet: starting.
          expect(textOf(t, 'task-agent-note'), 'The agent is starting on this task.');

          await assign(t, 'Agent', 'Me');
          expect(jobs.cancelled, ['job-1']);
          await assign(t, 'Me', 'Agent');
          expect(jobs.created, hasLength(1), reason: 'reassigning starts no run');
          expect((await core(t, (v) => loadTask(v, page))).agentRun, 'job-1');
        } finally {
          await closeAi(t, store);
        }
      });
    }

    appTest('desktop on this computer: the run answers into the task page and its status shows', (t) async {
      final tv = await pumpApp(t, mobile: false, empty: true);
      final store = await agentAi(t);
      try {
        final page = await taskPage(t, 'Plan the garden');
        await pumpTaskHost(t, tv, taskNote(page, 'Plan the garden'));
        await assign(t, 'Me', 'Agent');
        await settleUntil(t, () => store.tasks.isNotEmpty && store.tasks.first.status == AiStatus.done,
            timeout: const Duration(seconds: 30));
        final run = store.tasks.first;
        expect(run.node, tv.vault.deviceId);
        expect(run.page, page);
        expect((await core(t, (v) => loadTask(v, page))).agentRun, run.id);
        final texts = textsOf(await blocksOf(t, page));
        expect(texts, contains('ok'));
        await settleUntil(t, () => find.text('The agent finished; its answer is on this page.').evaluate().isNotEmpty);
        expect(textOf(t, 'task-agent-note'), 'The agent finished; its answer is on this page.');
      } finally {
        await closeAi(t, store);
      }
    });
  });
}
