// AI on the real core. The only AI that runs inside the app is the agent
// (BubblePaw, with its fake Claude Code extension in tests, see
// agent_test.dart); the Claude Code, Codex and DeepSeek runners were
// removed on 2026-10-22. Never calls a real AI.
// - settings saved by older builds, with the removed runners' keys, load;
// - Ask AI from the / menu on this computer: the agent answers into the
//   page;
// - a task queued on another device runs here and its status syncs back;
// - a row that names a removed runner is answered by the agent;
// - the MCP endpoint (through AiStore; its settings UI is gone): bearer token
//   required.

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
  // Real HTTP (the MCP endpoint), not the binding's stand-in client.
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
        'mcpEnabled': true,
        'mcpAddress': '127.0.0.1:47199',
        'agentEnabled': true,
      }));
    final s = AiSettings.load(file);
    expect(s.mcpEnabled, isTrue, reason: 'the other keys still load');
    expect(s.mcpAddress, '127.0.0.1:47199');
    expect(s.answersAskAi, agentPlatform);
    s.update((s) => s.mcpEnabled = false);
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
    expect(saved['mcpAddress'], '127.0.0.1:47199');
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

  appTest('a task queued on another device runs here; its status syncs back', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    final store = await agentAiStore(tester);
    try {
      await pairOther(tester, other);
      final page = tv.pages['Recipes']!;
      final t = await real(tester,
          () => aiCreateTask(vault: other.vault.raw, prompt: 'Say ok', page: page, after: null, runner: ''));
      expect(t.status, AiStatus.queued);
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

  // The endpoint has no settings UI any more (the MCP section left the Agent
  // page), so the test turns it on and off through AiStore.
  appTest('the MCP endpoint requires the token', (tester) async {
    await pumpApp(tester, mobile: false);
    final store = await aiStore(tester, settings: AiSettings()
      ..agentEnabled = false
      ..mcpAddress = '127.0.0.1:0');
    try {
      expect(store.mcpRunning, isFalse);
      await real(tester, () => store.setMcpEnabled(true));
      expect(store.mcpRunning, isTrue);
      final url = store.mcpUrl!;
      expect(url, startsWith('http://127.0.0.1:'));
      final endpointFile = File('${current.vault.root}/.vault/mcp/endpoint.json');
      expect(endpointFile.existsSync(), isTrue);

      final token = await real(tester, store.token);
      Future<(int, String)> post(String? bearer) async {
        final c = HttpClient();
        try {
          final req = await c.postUrl(Uri.parse(url));
          req.headers.contentType = ContentType.json;
          if (bearer != null) req.headers.set(HttpHeaders.authorizationHeader, 'Bearer $bearer');
          final bytes = utf8.encode(jsonEncode(
              {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call', 'params': {'name': 'list_pages', 'arguments': {}}}));
          // Match the native clients' framed requests. An early 401 can race
          // chunked body delivery and look like an unsolicited response.
          req.contentLength = bytes.length;
          req.persistentConnection = false;
          req.add(bytes);
          final res = await req.close();
          return (res.statusCode, await utf8.decoder.bind(res).join());
        } finally {
          c.close(force: true);
        }
      }

      final (denied, _) = await real(tester, () => post('wrong'));
      expect(denied, 401);
      final (ok, body) = await real(tester, () => post(token));
      expect(ok, 200);
      expect(body, contains('Garden plan'));

      await real(tester, () => store.setMcpEnabled(false));
      expect(store.mcpRunning, isFalse);
      expect(endpointFile.existsSync(), isFalse);
    } finally {
      await closeAi(tester, store);
    }
  });
}
