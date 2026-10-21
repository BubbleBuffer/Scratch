// Settings > AI > Agent > Default model lists the models of the default
// host ("Agent runs on"), not those of the host the AI tab is on. The other
// host's list is read through `AgentStore.debugHostModels`; the hub is a
// closed loopback port, so the tab's hub agent never opens.
// Each runner also has a tool set (integration spec phase 5), a vault
// setting next to its default model.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/settings/controls.dart';

import 'agent_test.dart' show fakeAgentSettings;
import 'ai_test.dart' show aiStore, closeAi;
import 'harness.dart';
import 'sync_helpers.dart';

Finder key(String k) => find.byKey(ValueKey(k));

const _local = [
  AgentModel(id: 'codex/local-a', name: 'Local A'),
  AgentModel(id: 'codex/local-b', name: 'Local B'),
];
const _hub = [AgentModel(id: 'codex/hub-x', name: 'Hub X')];

void main() {
  setUpAll(initCore);

  /// Default "This computer", the tab on the Hub, its models [_hub].
  Future<(AgentStore, AiSettings, File)> tabOnHub(
    WidgetTester t,
    TestVault tv,
  ) async {
    final file = File('${tv.dir.path}/default-model-settings.json');
    final settings = AiSettings.load(file);
    final fake = fakeAgentSettings();
    settings.update((s) => s
      ..agentEnabled = true
      ..agentHostOverride = AgentHost.local
      ..agentBunPath = fake.agentBunPath
      ..agentClaudeCodeDir = fake.agentClaudeCodeDir);
    final store = AgentStore.attach(tv.vault, settings);
    // The default model is a vault setting now (the row reads and writes it).
    await real(t, () => store.setVaultDefaultModel(AgentHost.local, 'codex/local-a'));
    final app = tv.app.settings;
    addTearDown(() => app
      ..clientUrl = null
      ..clientToken = null);
    store.useAppSettings(app);
    app.update((s) => s
      ..clientUrl = 'https://127.0.0.1:9'
      ..clientToken = 'test-token');
    await settle(t);
    await real(t, () => store.pickHost(AgentHost.hub));
    expect(store.host, AgentHost.hub);
    expect(store.defaultHost, AgentHost.local);
    store.models = _hub;
    return (store, settings, file);
  }

  Future<void> openAiSettings(WidgetTester t) async {
    await tapAndSettle(t, key('sidebar-settings'));
    await tapAndSettle(t, key('settings-nav-ai'));
    await real(t, () => Future<void>.delayed(const Duration(milliseconds: 50)));
    await settle(t);
  }

  appTest('Default model lists this computer\'s models while the tab is on '
      'the hub, and a pick is the default of new local chats', (t) async {
    final tv = await pumpApp(t, mobile: false);
    final asked = <AgentHost>[];
    AgentStore.debugHostModels = (h) async {
      asked.add(h);
      if (h == AgentHost.local) return _local;
      throw StateError('the tab host is not read here');
    };
    addTearDown(() => AgentStore.debugHostModels = null);
    final (store, settings, file) = await tabOnHub(t, tv);
    final ai = await aiStore(t, settings: settings);
    try {
      await openAiSettings(t);
      expect(asked, [AgentHost.local], reason: "the default host's list");
      await t.ensureVisible(key('agent-default-model'));
      final picker = t.widget<SelectButton<String>>(key('agent-default-model'));
      expect(picker.options, ['codex/local-a', 'codex/local-b']);
      expect(picker.value, 'codex/local-a');
      expect(store.modelLabel('codex/local-b'), 'Local B');
      expect(store.shownModels(), ['codex/hub-x'],
          reason: "the chat's picker keeps the tab's list");

      picker.onChanged!('codex/local-b');
      await settle(t);
      await settleUntil(
        t,
        () => store.vaultDefaults.models[AgentHost.local] == 'codex/local-b',
      );
      expect(store.vaultDefaults.models[AgentHost.local], 'codex/local-b');
      expect(AiSettings.load(file).agentModelOverrides, isEmpty,
          reason: 'the row writes the vault, not this device');
      expect(
        t.widget<SelectButton<String>>(key('agent-default-model')).value,
        'codex/local-b',
      );
      expect(asked, [AgentHost.local], reason: 'the list is kept');

      await real(t, () => store.pickHost(AgentHost.local));
      expect(store.host, AgentHost.local);
      store.models = _local;
      expect(store.modelOf(null), 'codex/local-b',
          reason: 'new local chats use the pick');
    } finally {
      await closeAi(t, ai);
    }
  });

  appTest('an unreachable default host shows the stored model and keeps it',
      (t) async {
    final tv = await pumpApp(t, mobile: false);
    AgentStore.debugHostModels = (h) async => throw StateError('unreachable');
    addTearDown(() => AgentStore.debugHostModels = null);
    final (store, settings, file) = await tabOnHub(t, tv);
    final ai = await aiStore(t, settings: settings);
    try {
      await openAiSettings(t);
      expect(key('agent-default-model'), findsNothing,
          reason: "the tab's hub models are not offered instead");
      await t.ensureVisible(key('agent-default-model-stored'));
      expect(
        t.widget<Text>(key('agent-default-model-stored')).data,
        'codex/local-a',
      );
      expect(key('agent-default-model-unavailable'), findsOneWidget);
      expect(store.defaultHostModels().unavailable, isTrue);
      expect(store.vaultDefaults.models[AgentHost.local], 'codex/local-a');
      expect(AiSettings.load(file).agentModelOverrides, isEmpty,
          reason: 'nothing overwrote the setting');
    } finally {
      await closeAi(t, ai);
    }
  });

  appTest('each runner offers the tool sets, Everything by default, and a '
      'pick is a vault setting', (t) async {
    final tv = await pumpApp(t, mobile: false);
    AgentStore.debugHostModels = (h) async => _local;
    addTearDown(() => AgentStore.debugHostModels = null);
    final (store, settings, _) = await tabOnHub(t, tv);
    final ai = await aiStore(t, settings: settings);
    try {
      await openAiSettings(t);
      for (final runner in ['this_device', 'hub']) {
        await t.ensureVisible(key('agent-tool-set-$runner'));
        final picker =
            t.widget<SelectButton<String>>(key('agent-tool-set-$runner'));
        expect(picker.options, ['read', 'write', 'everything']);
        expect(picker.value, 'everything', reason: 'the default');
      }
      expect(find.textContaining('Applies after the hub restarts.'),
          findsOneWidget);

      t.widget<SelectButton<String>>(key('agent-tool-set-hub')).onChanged!(
          'read');
      await settle(t);
      await settleUntil(
          t, () => store.vaultDefaults.toolSetFor(AgentHost.hub) == 'read');
      expect(
        await real(t, () => tv.vault.run((v) => v.agentToolSet(runner: 'hub'))),
        'read',
      );
      expect(store.vaultDefaults.toolSetFor(AgentHost.local), 'everything',
          reason: 'each runner has its own set');
      expect(
        t.widget<SelectButton<String>>(key('agent-tool-set-hub')).value,
        'read',
      );

      t.widget<SelectButton<String>>(key('agent-tool-set-hub')).onChanged!(
          'everything');
      await settle(t);
      await settleUntil(t, () => store.vaultDefaults.toolSets.isEmpty);
      expect(
        await real(t, () => tv.vault.run((v) => v.agentToolSet(runner: 'hub'))),
        'everything',
      );
    } finally {
      await closeAi(t, ai);
    }
  });
}
