// AI on the real core, with the fake agent (crates/mcp/examples/fake_agent.rs,
// built with `cargo build --release -p vault_mcp --examples`) in place of
// Claude Code, and a local HTTP server in place of DeepSeek. Never calls a
// real AI.
// - Ask AI from the / menu: the agent reads the page over the app's MCP
//   endpoint and its answer replaces the placeholder;
// - a task queued on another device runs here and its status syncs back;
// - a failing agent: the task fails, shows in Settings > AI, and retries;
// - Resolve with AI: a proposal that is applied only when accepted;
// - DeepSeek through its HTTP API;
// - the MCP endpoint from settings: bearer token required.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/ai/ai_store.dart';
import 'package:notes_app/src/rust/api/ai.dart';

import 'harness.dart';
import 'sync_helpers.dart';

/// The fake agent: NOTES_FAKE_AGENT, or `examples/` next to the bridge.
String fakeAgent() {
  final forced = Platform.environment['NOTES_FAKE_AGENT'];
  if (forced != null && forced.isNotEmpty) return forced;
  final lib = File(bridgeLibrary());
  final p = '${lib.parent.path}${Platform.pathSeparator}examples${Platform.pathSeparator}fake_agent${Platform.isWindows ? '.exe' : ''}';
  if (!File(p).existsSync()) fail('build the fake agent first: cargo build --release -p vault_mcp --examples ($p)');
  return p;
}

/// Gives the test vault's AI store [settings] (a fake Claude Code by
/// default) and closes it after [body].
Future<AiStore> aiStore(WidgetTester tester, {AiSettings? settings}) async {
  final s = settings ??
      (AiSettings()
        ..agentEnabled = false
        ..runner = AiRunner.claude
        ..claudePath = fakeAgent());
  AiStore.workDirOverride = '${current.dir.path}${Platform.pathSeparator}agent';
  final store = AiStore.attach(current.vault, current.app.feed, s);
  await real(tester, store.start);
  addTearDown(() => AiStore.workDirOverride = null);
  return store;
}

Future<void> closeAi(WidgetTester tester, AiStore store) async {
  await tester.pumpWidget(const SizedBox.shrink());
  await real(tester, store.close);
}

Future<void> pressKey(WidgetTester tester, LogicalKeyboardKey k) async {
  await tester.sendKeyEvent(k);
  await settle(tester);
}

List<String> texts(List<FlatBlock> blocks) => [for (final b in blocks) b.text];

void main() {
  setUpAll(initCore);
  // Real HTTP (the fake DeepSeek server, the MCP endpoint), not the
  // binding's stand-in client.
  setUp(() => HttpOverrides.global = null);

  appTest('Ask AI from the / menu: the agent answers into the page', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final store = await aiStore(tester);
    try {
      final page = tv.pages['Garden plan']!;
      await tapAndSettle(tester, find.text('Garden plan').last);
      final b = editableStarting('Some text about garden plan');
      await focusAt(tester, b, 'Some text about garden plan.'.length);
      await pressKey(tester, LogicalKeyboardKey.enter);
      await typeText(tester, '/ask');
      expect(key('slash-askai'), findsOneWidget);
      await pressKey(tester, LogicalKeyboardKey.enter);
      expect(key('ask-ai-field'), findsOneWidget);
      expect(textOf(tester, 'ask-ai-note'), contains('Claude Code answers on this device'));
      await enterField(tester, 'ask-ai-field', 'List what to plant');
      await tester.tap(key('ask-ai-send'));
      await settleUntil(tester, () => store.tasks.isNotEmpty && store.tasks.first.status == AiStatus.done,
          timeout: const Duration(seconds: 30));
      expect(store.tasks.single.status, AiStatus.done);
      final blocks = await blocksOf(tester, page);
      expect(texts(blocks), [
        'Some text about garden plan.',
        'Fake answer to: List what to plant',
        'Read over MCP: Garden plan',
      ]);
      expect(blocks[1].kind, Kinds.bulleted);
      // The editor shows the answer.
      await settleUntil(tester, () => editableWith('Read over MCP: Garden plan').evaluate().isNotEmpty);
      expect(editableWith('Read over MCP: Garden plan'), findsOneWidget);
      // The task is a row of the AI tasks collection.
      final coll = await real(tester, () => aiTasksCollection(vault: tv.vault.raw));
      expect(coll, isNotNull);
    } finally {
      await closeAi(tester, store);
    }
  });

  appTest('a task queued on another device runs here; its status syncs back', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    final store = await aiStore(tester);
    try {
      await pairOther(tester, other);
      final page = tv.pages['Recipes']!;
      final t = await real(tester,
          () => aiCreateTask(vault: other.vault.raw, prompt: 'Suggest a soup', page: page, after: null, runner: ''));
      expect(t.status, AiStatus.queued);
      await real(tester, () => other.vault.flush());
      await real(tester, () => other.client!.syncOnce());
      await settleUntil(tester, () => store.tasks.any((x) => x.id == t.id && x.status == AiStatus.done),
          timeout: const Duration(seconds: 30));
      expect(texts(await blocksOf(tester, page)).last, 'Read over MCP: Recipes');
      expect(store.tasks.firstWhere((x) => x.id == t.id).device, tv.vault.deviceId);

      await real(tester, () => tv.vault.flush());
      await real(tester, () => other.client!.syncOnce());
      final there = await real(tester, () => aiGetTask(vault: other.vault.raw, id: t.id));
      expect(there.status, AiStatus.done);
      final p = await real(tester, () => other.vault.loadPage(page));
      expect(p.blocks.last.text, 'Read over MCP: Recipes');
    } finally {
      await closeAi(tester, store);
      await other.close(tester);
    }
  });

  appTest('a failing agent fails the task; Settings > AI shows it and retries', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final store = await aiStore(tester);
    try {
      final page = tv.pages['Budget']!;
      await real(tester, () => store.ask('FAKE:fail please', page));
      await settleUntil(tester, () => store.tasks.isNotEmpty && store.tasks.first.status == AiStatus.failed,
          timeout: const Duration(seconds: 30));
      final t = store.tasks.single;
      expect(t.error, contains('code 3'));
      final last = (await blocksOf(tester, page)).last;
      expect(last.kind, Kinds.callout);
      expect(last.text, startsWith('AI task failed'));

      await tapAndSettle(tester, key('sidebar-settings'));
      await tapAndSettle(tester, key('settings-nav-ai'));
      expect(key('ai-task-${t.id}'), findsOneWidget);
      expect(find.text('Failed'), findsOneWidget);
      expect(find.textContaining('asked to fail'), findsOneWidget);

      // Retry with a runner that is off: it stays queued.
      store.settings.update((s) => s.runner = AiRunner.none);
      await tester.ensureVisible(key('ai-retry-${t.id}'));
      await settle(tester);
      await tapAndSettle(tester, key('ai-retry-${t.id}'));
      await settleUntil(tester, () => store.tasks.first.status == AiStatus.queued);
      expect(find.text('Queued'), findsOneWidget);
    } finally {
      await closeAi(tester, store);
    }
  });

  appTest('DeepSeek answers through its HTTP API', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final server = (await tester.runAsync(() => HttpServer.bind(InternetAddress.loopbackIPv4, 0)))!;
    final seen = <Map<String, dynamic>>[];
    String? auth;
    server.listen((req) async {
      auth = req.headers.value(HttpHeaders.authorizationHeader);
      final body = jsonDecode(await utf8.decoder.bind(req).join()) as Map<String, dynamic>;
      seen.add(body);
      req.response.headers.contentType = ContentType.json;
      req.response.write(jsonEncode({
        'choices': [
          {
            'message': {'role': 'assistant', 'content': '- Rent\n- Food'}
          }
        ]
      }));
      await req.response.close();
    });
    final store = await aiStore(tester,
        settings: AiSettings()
          ..agentEnabled = false
          ..runner = AiRunner.deepseek
          ..deepSeekKey = 'test-key'
          ..deepSeekUrl = 'http://127.0.0.1:${server.port}');
    try {
      final page = tv.pages['Budget']!;
      await real(tester, () => store.ask('List the costs', page));
      await settleUntil(tester, () => store.tasks.isNotEmpty && store.tasks.first.status == AiStatus.done,
          timeout: const Duration(seconds: 30));
      expect(texts(await blocksOf(tester, page)), ['Some text about budget.', 'Rent', 'Food']);
      expect(auth, 'Bearer test-key');
      expect(seen.single['model'], defaultDeepSeekModel);
      final user = (seen.single['messages'] as List).last['content'] as String;
      expect(user, contains('List the costs'));
      expect(user, contains('Some text about budget.'));
    } finally {
      await closeAi(tester, store);
      await tester.runAsync(() => server.close(force: true));
    }
  });

  appTest('the MCP endpoint from settings requires the token', (tester) async {
    await pumpApp(tester, mobile: false);
    final store = await aiStore(tester, settings: AiSettings()
      ..agentEnabled = false
      ..mcpAddress = '127.0.0.1:0');
    try {
      await tapAndSettle(tester, key('sidebar-settings'));
      await tapAndSettle(tester, key('settings-nav-ai'));
      expect(key('ai-mcp-url'), findsNothing);
      await tester.tap(key('ai-mcp-toggle'));
      await settleUntil(tester, () => key('ai-mcp-url').evaluate().isNotEmpty);
      final url = textOf(tester, 'ai-mcp-url');
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

      await tester.tap(key('ai-mcp-toggle'));
      await settleUntil(tester, () => key('ai-mcp-url').evaluate().isEmpty);
      expect(store.mcpRunning, isFalse);
      expect(endpointFile.existsSync(), isFalse);
    } finally {
      await closeAi(tester, store);
    }
  });
}
