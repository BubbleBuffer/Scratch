// MCP servers in Settings > Connectors > Tools for the agent (desktop): add,
// edit and remove save in this device's AI settings and reach the engine; each
// server's state and
// a refused list show in the section; an old mcp.toml is imported once.
// The engine's MCP calls go through AgentStore's debug hooks.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/settings/settings_kit.dart';

import 'agent_test.dart' show fakeAgentSettings, openAgentStore;
import 'ai_test.dart' show aiStore, closeAi;
import 'harness.dart';
import 'sync_helpers.dart';

/// Device AI settings in [file] that open the fake agent, with [servers].
AiSettings _settings(File file, Map<String, Map<String, dynamic>> servers) {
  final fake = fakeAgentSettings();
  return AiSettings.load(file)
    ..agentEnabled = fake.agentEnabled
    ..agentModelOverrides.addAll(fake.agentModelOverrides)
    ..agentBunPath = fake.agentBunPath
    ..agentClaudeCodeDir = fake.agentClaudeCodeDir
    ..agentMcpServers = servers;
}

/// The servers are on Settings > Connectors, in "Tools for the agent".
Future<void> _openAgentSettings(WidgetTester t) async {
  await tapAndSettle(t, key('sidebar-settings'));
  await tapAndSettle(t, key('settings-nav-connectors'));
  await _tap(t, 'connectors-tools');
  await t.ensureVisible(key('agent-mcp-servers'));
  await settle(t);
}

Future<void> _tap(WidgetTester t, String k) async {
  await t.ensureVisible(key(k));
  await tapAndSettle(t, key(k));
}

Future<void> _enter(WidgetTester t, String k, String text) async {
  await t.ensureVisible(key(k));
  await enterField(t, k, text);
}

/// The label of a status pill (SStatus) found by key.
String _pill(WidgetTester t, String k) => t
    .widget<Text>(find.descendant(of: key(k), matching: find.byType(Text)))
    .data!;

void _resetHooks() {
  AgentStore.debugSetMcpServers = null;
  AgentStore.debugMcpStatus = null;
  AgentStore.debugMcpImport = null;
}

void main() {
  setUpAll(initCore);

  test('the server list is saved per device with unknown keys', () {
    final dir = Directory.systemTemp.createTempSync('notes-mcp-settings-');
    addTearDown(() => dir.deleteSync(recursive: true));
    final file = File('${dir.path}/ai_settings.json');
    final s = AiSettings.load(file);
    expect(s.agentMcpServers, isEmpty);
    s.update(
      (s) => s.agentMcpServers = {
        'files': {
          'command': 'npx',
          'args': ['-y', '@example/files'],
          'enabled': true,
          'call_timeout_secs': 60,
        },
      },
    );
    expect(AiSettings.load(file).agentMcpServers, {
      'files': {
        'command': 'npx',
        'args': ['-y', '@example/files'],
        'enabled': true,
        'call_timeout_secs': 60,
      },
    });
  });

  appTest('MCP servers: add, edit and remove save in the AI settings', (
    t,
  ) async {
    final sent = <String>[];
    AgentStore.debugSetMcpServers = (json) async => sent.add(json);
    AgentStore.debugMcpStatus = () async => const [];
    addTearDown(_resetHooks);
    final tv = await pumpApp(t, mobile: false);
    final file = File('${tv.dir.path}/mcp-settings.json');
    final settings = _settings(file, {
      'docs': {
        'url': 'https://hub.example.com/mcp',
        'enabled': true,
        'startup_timeout_secs': 30,
      },
    });
    await openAgentStore(t, settings: settings);
    final ai = await aiStore(t, settings: settings);
    try {
      expect(
        sent.last,
        contains('"docs"'),
        reason: 'the engine gets the list right after it opens',
      );
      await _openAgentSettings(t);
      expect(key('agent-mcp-server-docs'), findsOneWidget);

      // Add: a name that isn't taken and a command are required.
      await _tap(t, 'agent-mcp-add');
      await _tap(t, 'agent-mcp-save');
      expect(find.text('Give the server a name.'), findsOneWidget);
      await _enter(t, 'agent-mcp-name', 'docs');
      await _tap(t, 'agent-mcp-save');
      expect(find.text('A server named docs already exists.'), findsOneWidget);
      await _enter(t, 'agent-mcp-name', 'files');
      await _tap(t, 'agent-mcp-save');
      expect(find.text('Enter the command that starts it.'), findsOneWidget);
      await _enter(t, 'agent-mcp-command', 'npx');
      await _enter(t, 'agent-mcp-args', '-y\n@example/files\n');
      await _enter(t, 'agent-mcp-env', 'EXAMPLE_ROOT=/tmp/files');
      await _tap(t, 'agent-mcp-save');
      expect(key('agent-mcp-editor'), findsNothing);
      expect(AiSettings.load(file).agentMcpServers['files'], {
        'enabled': true,
        'command': 'npx',
        'args': ['-y', '@example/files'],
        'env': {'EXAMPLE_ROOT': '/tmp/files'},
      });
      expect(sent.last, contains('"files"'), reason: 'applied at once');

      // Edit a URL server: headers take env var names; unknown keys stay.
      await _tap(t, 'agent-mcp-edit-docs');
      expect(t.widget<SSegmented<bool>>(key('agent-mcp-type')).value, isTrue);
      await _enter(t, 'agent-mcp-headers', 'Authorization=EXAMPLE');
      await _tap(t, 'agent-mcp-save');
      expect(
        find.text('EXAMPLE must end in _API_KEY, _KEY or _TOKEN.'),
        findsOneWidget,
      );
      await _enter(t, 'agent-mcp-headers', 'Authorization=EXAMPLE_TOKEN');
      await _tap(t, 'agent-mcp-save');
      expect(AiSettings.load(file).agentMcpServers['docs'], {
        'url': 'https://hub.example.com/mcp',
        'enabled': true,
        'startup_timeout_secs': 30,
        'headers_from': {'Authorization': 'EXAMPLE_TOKEN'},
      });

      // Edit the command server into a URL server.
      await _tap(t, 'agent-mcp-edit-files');
      await _tap(t, 'agent-mcp-type-url');
      await _enter(t, 'agent-mcp-url', 'https://files.example.com/mcp');
      await _tap(t, 'agent-mcp-save');
      expect(AiSettings.load(file).agentMcpServers['files'], {
        'enabled': true,
        'url': 'https://files.example.com/mcp',
      });

      // Turn off, then remove.
      await _tap(t, 'agent-mcp-enabled-files');
      expect(AiSettings.load(file).agentMcpServers['files']?['enabled'], false);
      // Remove asks first; the confirm button is labelled "Remove server".
      await _tap(t, 'agent-mcp-remove-files');
      expect(find.text('Remove “files”?'), findsOneWidget);
      await tapAndSettle(t, find.text('Remove server'));
      expect(key('agent-mcp-server-files'), findsNothing);
      expect(AiSettings.load(file).agentMcpServers.keys, ['docs']);
      expect(sent.last, isNot(contains('"files"')));
    } finally {
      await closeAi(t, ai);
    }
  });

  appTest('MCP servers: a failed server and a refused list show why', (
    t,
  ) async {
    AgentStore.debugSetMcpServers = (_) async =>
        throw 'server files: url must be https';
    AgentStore.debugMcpStatus = () async => const [
      AgentMcpServer(name: 'docs', state: 'running', tools: 3),
      AgentMcpServer(
        name: 'files',
        state: 'failed',
        tools: 0,
        error: 'spawn npx: program not found',
      ),
    ];
    addTearDown(_resetHooks);
    final tv = await pumpApp(t, mobile: false);
    final file = File('${tv.dir.path}/mcp-status.json');
    final settings = _settings(file, {
      'docs': {'url': 'https://hub.example.com/mcp', 'enabled': true},
      'files': {'command': 'npx', 'enabled': true},
    });
    final store = await openAgentStore(t, settings: settings);
    final ai = await aiStore(t, settings: settings);
    try {
      expect(store.mcpError, 'server files: url must be https');
      await _openAgentSettings(t);
      expect(_pill(t, 'agent-mcp-status-docs'), 'Running, 3 tools');
      // The pill says "Failed"; the reason is the row's error line.
      expect(_pill(t, 'agent-mcp-status-files'), 'Failed');
      expect(
        find.text('Failed: spawn npx: program not found'),
        findsOneWidget,
      );
      expect(
        find.textContaining('server files: url must be https'),
        findsOneWidget,
      );
      expect(key('agent-mcp-note'), findsNothing);
    } finally {
      await closeAi(t, ai);
    }
  });

  appTest('mcp.toml is imported once, keeping servers already set', (
    t,
  ) async {
    // Like the bridge: the file is renamed once imported.
    String? toml =
        '{"files": {"command": "npx", "args": ["-y", "@example/files"]},'
        ' "docs": {"url": "https://other.example.com/mcp"}}';
    final tv = await pumpApp(t, mobile: false);
    final calls = <String>[];
    AgentStore.debugMcpImport = (dir) async {
      calls.add(dir);
      final json = toml;
      toml = null;
      return json;
    };
    addTearDown(_resetHooks);
    final file = File('${tv.dir.path}/import-settings.json');
    final settings = AiSettings.load(file)
      ..agentEnabled = false
      ..agentMcpServers = {
        'docs': {'url': 'https://hub.example.com/mcp', 'enabled': true},
      };
    // The app's own start may already have imported (nothing there).
    final store = AgentStore.attach(tv.vault, settings)..debugForgetMcpImport();
    await real(t, store.importMcpFile);
    await real(t, store.importMcpFile);
    expect(calls, [file.parent.path], reason: 'once per start');
    final saved = AiSettings.load(file).agentMcpServers;
    expect(saved['docs'], {
      'url': 'https://hub.example.com/mcp',
      'enabled': true,
    }, reason: 'a taken name keeps the existing server');
    expect(saved['files'], {
      'command': 'npx',
      'args': ['-y', '@example/files'],
    });

    // A later start: the bridge renamed the file, so nothing changes.
    store.debugForgetMcpImport();
    await real(t, store.importMcpFile);
    expect(calls, hasLength(2));
    expect(AiSettings.load(file).agentMcpServers, saved);
  });
}
