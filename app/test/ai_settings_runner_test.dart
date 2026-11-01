// Settings > Agent follows "Where it runs" (the vault's synced runner). On
// desktop the hub's model and tool set show only while the hub is chosen,
// and the computer's model, tool set and access only while This computer is
// chosen (its MCP servers are on Settings > Connectors). A phone that chose
// This computer is told it uses the hub. Settings > Connectors shows what
// the hub may use on this computer.
//
// No Hub is contacted: the hub's model lists are fakes (debugHostModels) and
// the hub address is a closed loopback port.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/settings/settings_kit.dart';

import 'agent_test.dart' show fakeAgentSettings;
import 'ai_test.dart' show aiStore, closeAi;
import 'harness.dart';
import 'sync_helpers.dart';

Finder key(String k) => find.byKey(ValueKey(k));

bool _has(Finder f) => f.evaluate().isNotEmpty;

const _local = [AgentModel(id: 'codex/local-a', name: 'Local A')];
const _hub = [
  AgentModel(id: 'codex/hub-x', name: 'Hub X'),
  AgentModel(id: 'codex/hub-y', name: 'Hub Y'),
];

/// The vault's runner set to [runner], on a signed-in desktop whose hub is a
/// closed loopback port (the hub's list comes from the fakes).
Future<(AgentStore, AiSettings)> _desktop(
  WidgetTester t,
  TestVault tv, {
  required AgentHost runner,
}) async {
  final fake = fakeAgentSettings();
  final settings = AiSettings.load(File('${tv.dir.path}/runner-settings.json'));
  settings.update(
    (s) => s
      ..agentEnabled = true
      ..agentBunPath = fake.agentBunPath
      ..agentClaudeCodeDir = fake.agentClaudeCodeDir,
  );
  final store = AgentStore.attach(tv.vault, settings);
  final app = tv.app.settings;
  addTearDown(
    () => app
      ..clientUrl = null
      ..clientToken = null,
  );
  store.useAppSettings(app);
  app.update(
    (s) => s
      ..clientUrl = 'https://127.0.0.1:9'
      ..clientToken = 'test-token',
  );
  await settle(t);
  await real(t, () => store.setVaultDefaultHost(runner));
  await settle(t);
  return (store, settings);
}

Future<void> _openAiSettings(WidgetTester t) async {
  await tapAndSettle(t, key('sidebar-settings'));
  await tapAndSettle(t, key('settings-nav-agent'));
  await real(t, () => Future<void>.delayed(const Duration(milliseconds: 50)));
  await settle(t);
}

void _resetHostFakes() {
  AgentStore.debugHostModels = null;
  AgentStore.debugHostDefaultModel = null;
}

void main() {
  setUpAll(initCore);

  appTest('desktop, Hub chosen: the picker offers the hub models and its default, '
      'the computer settings do not', (t) async {
    final tv = await pumpApp(t, mobile: false);
    AgentStore.debugHostModels = (h) async => h == AgentHost.hub ? _hub : _local;
    AgentStore.debugHostDefaultModel = 'codex/hub-y';
    addTearDown(_resetHostFakes);
    final (store, settings) = await _desktop(t, tv, runner: AgentHost.hub);
    final ai = await aiStore(t, settings: settings);
    try {
      await _openAiSettings(t);
      expect(store.host, AgentHost.hub);
      // This machine has no hub, so the open fails, but the hub's list is read
      // on its own connection (debugHostModels here): the picker is filled.
      await settleUntil(t, () {
        if (!_has(key('agent-hub-model'))) return false;
        return t
            .widget<SSelect<String?>>(key('agent-hub-model'))
            .options
            .length >
            1;
      });
      expect(key('agent-hub-unreachable'), findsNothing);
      await t.ensureVisible(key('agent-hub-model'));
      final picker = t.widget<SSelect<String?>>(key('agent-hub-model'));
      expect(picker.options, [null, 'codex/hub-x', 'codex/hub-y']);
      expect(picker.labelOf(null), 'Hub default (Hub Y)');
      expect(picker.labelOf('codex/hub-x'), 'Hub X');
      expect(picker.labelOf('codex/hub-y'), 'Hub Y');

      await t.ensureVisible(key('agent-tool-set-hub'));
      expect(key('agent-tool-set-hub'), findsOneWidget);
      expect(key('agent-tool-set-this_device'), findsNothing);

      expect(key('agent-computer-model'), findsNothing);
      expect(key('agent-computer-model-stored'), findsNothing);
      expect(key('agent-desktop-tools'), findsNothing);
      expect(key('agent-bun-path'), findsNothing);
      expect(key('agent-claude-code-dir'), findsNothing);
      // The MCP servers link is in this computer's settings only.
      expect(key('agent-mcp-link'), findsNothing);
      expect(t.takeException(), isNull);
    } finally {
      await closeAi(t, ai);
    }
  });

  appTest('desktop, This computer chosen: the computer settings show, the hub '
      'model and hub tool set do not', (t) async {
    final tv = await pumpApp(t, mobile: false);
    AgentStore.debugHostModels = (h) async => h == AgentHost.hub ? _hub : _local;
    AgentStore.debugHostDefaultModel = 'codex/hub-x';
    addTearDown(_resetHostFakes);
    final (store, settings) = await _desktop(t, tv, runner: AgentHost.local);
    await real(t, () => store.setVaultDefaultModel(AgentHost.local, 'codex/local-a'));
    final ai = await aiStore(t, settings: settings);
    try {
      await _openAiSettings(t);
      await settleUntil(t, () {
        if (!_has(key('agent-computer-model'))) return false;
        return t
            .widget<SSelect<String>>(key('agent-computer-model'))
            .options
            .isNotEmpty;
      });
      expect(store.host, AgentHost.local);

      await t.ensureVisible(key('agent-computer-model'));
      final picker = t.widget<SSelect<String>>(key('agent-computer-model'));
      expect(picker.options, contains('codex/local-a'));
      expect(picker.value, 'codex/local-a');

      await t.ensureVisible(key('agent-tool-set-this_device'));
      expect(key('agent-tool-set-this_device'), findsOneWidget);
      expect(key('agent-desktop-tools'), findsOneWidget);
      expect(key('agent-bun-path'), findsOneWidget);
      expect(key('agent-claude-code-dir'), findsOneWidget);
      expect(key('agent-mcp-link'), findsOneWidget);

      expect(key('agent-hub-model'), findsNothing);
      expect(key('agent-hub-unreachable'), findsNothing);
      expect(key('agent-tool-set-hub'), findsNothing);
      expect(t.takeException(), isNull);

      // The MCP servers themselves are on Connectors, in its tools panel.
      await tapAndSettle(t, key('settings-nav-connectors'));
      await tapAndSettle(t, key('connectors-tools'));
      expect(mcpServersAnchor.currentContext, isNotNull);
    } finally {
      await closeAi(t, ai);
    }
  });

  // The Ask AI runner lines ("Ask AI runs on …") are gone: the runner is
  // shown once, under "Where it runs". Under either runner there are still no
  // "Answered by", program paths or DeepSeek fields (removed 2026-10-22).
  for (final runner in [AgentHost.hub, AgentHost.local]) {
    appTest('desktop, ${runner.name} chosen: no Ask AI runner line, program paths or DeepSeek fields', (t) async {
      final tv = await pumpApp(t, mobile: false);
      AgentStore.debugHostModels =
          (h) async => h == AgentHost.hub ? _hub : _local;
      addTearDown(_resetHostFakes);
      final (_, settings) = await _desktop(t, tv, runner: runner);
      final ai = await aiStore(t, settings: settings);
      try {
        await _openAiSettings(t);
        Finder any(String k) =>
            find.byKey(ValueKey(k), skipOffstage: false);
        expect(key('agent-host'), findsOneWidget);
        for (final gone in [
          'ai-runs-on-hub',
          'ai-runs-on-this-computer',
          'ai-runner',
          'ai-path-claude',
          'ai-path-codex',
          'ai-deepseek-key',
          'ai-deepseek-save',
          'ai-deepseek-remove',
        ]) {
          expect(any(gone), findsNothing, reason: gone);
        }
        for (final text in [
          'Answered by',
          'Claude Code program',
          'Codex program',
          'DeepSeek API key',
          'API address',
        ]) {
          expect(find.text(text, skipOffstage: false), findsNothing, reason: text);
        }
        expect(t.takeException(), isNull);
      } finally {
        await closeAi(t, ai);
      }
    });
  }

  appTest('phone, This computer chosen: the settings say phones use the hub, '
      'and Hub chosen does not', (t) async {
    final tv = await pumpApp(t, mobile: true);
    final store = AgentStore.of(tv.app);
    await real(t, () => store.setVaultDefaultHost(AgentHost.local));
    await openMobileSettings(t);
    await tapAndSettle(t, key('settings-nav-agent'));

    const phoneNote = 'Your computers use their own agent. This phone uses the hub.';
    expect(key('agent-host'), findsOneWidget);
    expect(find.text(phoneNote), findsOneWidget);
    expect(key('agent-phone-uses-hub'), findsOneWidget);
    expect(key('agent-computer-model'), findsNothing);
    expect(t.takeException(), isNull);

    await real(t, () => store.setVaultDefaultHost(AgentHost.hub));
    await settle(t);
    expect(key('agent-phone-uses-hub'), findsNothing);
    expect(find.text(phoneNote), findsNothing);
    expect(t.takeException(), isNull);
  });

  appTest('desktop, Hub chosen but unreachable: the last list the hub reported '
      'shows, and the picker offers it', (t) async {
    final tv = await pumpApp(t, mobile: false);
    AgentStore.debugHostModels = (h) async => throw StateError('hub unreachable');
    addTearDown(_resetHostFakes);
    final (store, settings) = await _desktop(t, tv, runner: AgentHost.hub);
    settings.update(
      (s) => s.hubModels = HubModelList(
        models: [(id: 'codex/cached', name: 'Cached model')],
        defaultModel: 'codex/cached',
      ),
    );
    final ai = await aiStore(t, settings: settings);
    try {
      await _openAiSettings(t);
      await settleUntil(t, () => _has(key('agent-hub-unreachable')));
      expect(
        find.text(
          "Can't reach the hub. These are the models it last listed.",
        ),
        findsOneWidget,
      );
      expect(store.runnerModels(AgentHost.hub).unreachable, isTrue);

      await t.ensureVisible(key('agent-hub-model'));
      final picker = t.widget<SSelect<String?>>(key('agent-hub-model'));
      expect(picker.options, [null, 'codex/cached']);
      expect(picker.labelOf(null), startsWith('Hub default ('));
      expect(key('agent-tool-set-hub'), findsOneWidget);
      expect(t.takeException(), isNull);
    } finally {
      await closeAi(t, ai);
    }
  });

  appTest('Settings > Connectors with hub access on shows what the hub may use '
      'on this computer', (t) async {
    HostRelayLink.debugStatus = () async =>
        const AgentHostRelayStatus(state: 'connected');
    addTearDown(() => HostRelayLink.debugStatus = null);
    final tv = await pumpApp(t, mobile: false);
    final store = AgentStore.of(tv.app);
    // An http hub: the relay never opens a socket in the test.
    tv.app.settings.update(
      (s) => s
        ..clientUrl = 'http://hub.example.com'
        ..clientToken = 'test-token'
        ..clientDeviceLabel = 'Work PC'
        ..hubUsesThisComputer = true,
    );
    await tapAndSettle(t, key('sidebar-settings'));
    await tapAndSettle(t, key('settings-nav-connectors'));
    await tapAndSettle(t, key('connectors-hub'));

    expect(key('host-relay-name'), findsOneWidget);
    expect(key('host-relay-tools'), findsOneWidget);
    expect(find.text('What it shares'), findsOneWidget);
    expect(find.text(hostRelayToolsLine(store)), findsOneWidget);
    expect(
      hostRelayToolsLine(store),
      startsWith('The vault'),
    );
    expect(key('host-relay-mcp-link'), findsOneWidget);
    expect(t.takeException(), isNull);
  });
}
