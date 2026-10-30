// AI on the real core. The only AI that runs inside the app is the agent
// (BubblePaw, with its fake Claude Code extension in tests, see
// agent_test.dart); the Claude Code, Codex and DeepSeek runners were
// removed on 2026-10-22. Never calls a real AI.
// - settings saved by older builds, with the removed runners' keys, load;
// - Ask AI from the / menu on this computer: the agent answers into the
//   page;
// - a legacy unpinned task queued on another device runs here and syncs back;
// - a row that names a removed runner is answered by the agent;

import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/ai/ai_store.dart';
import 'package:notes_app/src/rust/api/ai.dart';

import 'agent_test.dart' show fakeAgentSettings, openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

/// Gives the test vault's AI store [settings] (default: the agent off) and
/// starts it; close it with [closeAi].
Future<AiStore> aiStore(WidgetTester tester, {AiSettings? settings}) async {
  final s = settings ?? (AiSettings()..agentEnabled = false);
  final store = AiStore.attach(current.vault, current.app.feed, s);
  await real(tester, store.start);
  return store;
}

Future<void> closeAi(WidgetTester tester, AiStore store) async {
  await tester.pumpWidget(const SizedBox.shrink());
  await real(tester, store.close);
}

/// An AI store whose agent (BubblePaw's fake extension, which answers "ok")
/// answers Ask AI on this computer.
Future<AiStore> agentAiStore(WidgetTester tester) async {
  final settings = fakeAgentSettings();
  await openAgentStore(tester, settings: settings);
  return aiStore(tester, settings: settings);
}

Future<void> pressKey(WidgetTester tester, LogicalKeyboardKey k) async {
  await tester.sendKeyEvent(k);
  await settle(tester);
}

List<String> texts(List<FlatBlock> blocks) => [for (final b in blocks) b.text];

bool _ended(AiTask t) => !AiStatus.active(t.status);

void main() {
  setUpAll(initCore);
  // Real HTTP, not the binding's stand-in client.
  setUp(() => HttpOverrides.global = null);

  test('settings from before 2026-10-22 with the removed runners load; '
      'their keys are dropped on save', () {
    final dir = Directory.systemTemp.createTempSync('notes-ai-settings-');
    addTearDown(() => dir.deleteSync(recursive: true));
    final file = File('${dir.path}/ai_settings.json')
      ..writeAsStringSync(jsonEncode({
        'runner': 'deepseek',
        'claudePath': r'C:\Tools\claude.exe',
        'codexPath': '/usr/bin/codex',
        'deepSeekKey': 'old-key',
        'deepSeekModel': 'deepseek-chat',
        'deepSeekUrl': 'https://api.deepseek.com',
        'timeoutMinutes': 5,
        'agentEnabled': true,
      }));
    final s = AiSettings.load(file);
    expect(s.answersAskAi, agentPlatform, reason: 'the other keys still load');
    s.update((s) => s.desktopTools = false);
    final saved = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
    for (final k in [
      'runner',
      'claudePath',
      'codexPath',
      'deepSeekKey',
      'deepSeekModel',
      'deepSeekUrl',
      'timeoutMinutes',
    ]) {
      expect(saved.containsKey(k), isFalse, reason: k);
    }
    expect(saved['desktopTools'], isFalse);
  });

  appTest('Ask AI from the / menu on this computer: the agent answers into the page', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final store = await agentAiStore(tester);
    try {
      expect(store.runsOn, AgentHost.local);
      final page = tv.pages['Garden plan']!;
      await tapAndSettle(tester, find.text('Garden plan').last);
      final b = editableStarting('Some text about garden plan');
      await focusAt(tester, b, 'Some text about garden plan.'.length);
      await pressKey(tester, LogicalKeyboardKey.enter);
      await typeText(tester, '/ask');
      expect(key('slash-askai'), findsOneWidget);
      await pressKey(tester, LogicalKeyboardKey.enter);
      expect(key('ask-ai-field'), findsOneWidget);
      expect(textOf(tester, 'ask-ai-note'), 'The agent answers on this device and writes into the page.');
      await enterField(tester, 'ask-ai-field', 'Say ok');
      await tester.tap(key('ask-ai-send'));
      await settleUntil(tester, () => store.tasks.isNotEmpty && _ended(store.tasks.first),
          timeout: const Duration(seconds: 30));
      final t = store.tasks.single;
      expect(t.status, AiStatus.done, reason: t.error);
      expect(t.node, tv.vault.deviceId, reason: 'pinned to this computer');
      expect(texts(await blocksOf(tester, page)), ['Some text about garden plan.', 'ok']);
    } finally {
      await closeAi(tester, store);
    }
  });

  appTest('a legacy unpinned task queued on another device runs here; its status syncs back', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    final store = await agentAiStore(tester);
    try {
      await pairOther(tester, other);
      final page = tv.pages['Recipes']!;
      final t = await real(tester,
          () => aiCreateTask(vault: other.vault.raw, prompt: 'Say ok', page: page, after: null, runner: ''));
      expect(t.status, AiStatus.queued);
      expect(t.node, other.vault.deviceId, reason: 'new bridge rows are pinned');
      // Reproduce a row from before creation always pinned its runner.
      await real(tester, () => other.vault.raw.removeProperty(page: t.id, key: 'node'));
      await real(tester, () => other.vault.flush());
      await real(tester, () => other.client!.syncOnce());
      await settleUntil(tester, () => store.tasks.any((x) => x.id == t.id && _ended(x)),
          timeout: const Duration(seconds: 30));
      final here = store.tasks.firstWhere((x) => x.id == t.id);
      expect(here.status, AiStatus.done, reason: here.error);
      expect(here.device, tv.vault.deviceId);
      expect(texts(await blocksOf(tester, page)).last, 'ok');

      await real(tester, () => tv.vault.flush());
      await real(tester, () => other.client!.syncOnce());
      final there = await real(tester, () => aiGetTask(vault: other.vault.raw, id: t.id));
      expect(there.status, AiStatus.done);
    } finally {
      await closeAi(tester, store);
      await other.close(tester);
    }
  });

  // Rows created before 2026-10-22 may name Claude Code, Codex or DeepSeek:
  // the agent answers them like any other.
  for (final runner in ['claude', 'codex', 'deepseek']) {
    appTest('a row naming the removed runner $runner is answered by the agent', (tester) async {
      final tv = await pumpApp(tester, mobile: false);
      final store = await agentAiStore(tester);
      try {
        final page = tv.pages['Budget']!;
        final t = await real(tester,
            () => aiCreateTask(vault: tv.vault.raw, prompt: 'Say ok', page: page, after: null, runner: runner));
        await settleUntil(tester, () => store.tasks.any((x) => x.id == t.id && _ended(x)),
            timeout: const Duration(seconds: 30));
        final done = store.tasks.firstWhere((x) => x.id == t.id);
        expect(done.runner, runner);
        expect(done.status, AiStatus.done, reason: done.error);
        expect(texts(await blocksOf(tester, page)).last, 'ok');
      } finally {
        await closeAi(tester, store);
      }
    });
  }
}
