// Settings > AI > Agent: the model and tool set of the runner chosen under
// "Run AI on" (the vault's, synced). The computer's model is read from this
// computer while the tab is on the hub; the hub is a closed loopback port, so
// the tab's hub agent never opens. Each runner has a tool set (integration
// spec phase 5), a vault setting, and only the chosen runner's set is shown.

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

  /// Run AI on: This computer (the default), the tab on the Hub, its models
  /// [_hub].
  Future<(AgentStore, AiSettings, File)> tabOnHub(
    WidgetTester t,
    TestVault tv,
  ) async {
    final file = File('${tv.dir.path}/default-model-settings.json');
    final settings = AiSettings.load(file);
    final fake = fakeAgentSettings();
    settings.update((s) => s
      ..agentEnabled = true
      ..agentBunPath = fake.agentBunPath
      ..agentClaudeCodeDir = fake.agentClaudeCodeDir);
    final store = AgentStore.attach(tv.vault, settings);
    // The default model is a vault setting (the row reads and writes it).
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

  appTest('the computer model lists this computer\'s models while the tab is on '
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
      expect(asked, [AgentHost.local], reason: 'the chosen runner\'s list');
      await t.ensureVisible(key('agent-computer-model'));
      final picker = t.widget<SelectButton<String>>(key('agent-computer-model'));
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
        t.widget<SelectButton<String>>(key('agent-computer-model')).value,
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

  appTest('an unreachable computer shows the stored model and keeps it',
      (t) async {
    final tv = await pumpApp(t, mobile: false);
    AgentStore.debugHostModels = (h) async => throw StateError('unreachable');
    addTearDown(() => AgentStore.debugHostModels = null);
    final (store, settings, file) = await tabOnHub(t, tv);
    final ai = await aiStore(t, settings: settings);
    try {
      await openAiSettings(t);
      expect(key('agent-computer-model'), findsNothing,
          reason: "the tab's hub models are not offered instead");
      await t.ensureVisible(key('agent-computer-model-stored'));
      expect(
        t.widget<Text>(key('agent-computer-model-stored')).data,
        'codex/local-a',
      );
      expect(find.textContaining("Can't list this computer's models"),
          findsOneWidget);
      expect(store.defaultHostModels().unavailable, isTrue);
      expect(store.vaultDefaults.models[AgentHost.local], 'codex/local-a');
      expect(AiSettings.load(file).agentModelOverrides, isEmpty,
          reason: 'nothing overwrote the setting');
    } finally {
      await closeAi(t, ai);
    }
  });

  appTest('the chosen runner\'s tool set shows, Everything by default, and a '
      'pick is a vault setting per runner', (t) async {
    final tv = await pumpApp(t, mobile: false);
    AgentStore.debugHostModels = (h) async => _local;
    addTearDown(() => AgentStore.debugHostModels = null);
    final (store, settings, _) = await tabOnHub(t, tv);
    final ai = await aiStore(t, settings: settings);
    try {
      await openAiSettings(t);
      // This computer is chosen: its set shows, the hub's does not.
      await t.ensureVisible(key('agent-tool-set-this_device'));
      var picker =
          t.widget<SelectButton<String>>(key('agent-tool-set-this_device'));
      expect(picker.options, ['read', 'write', 'everything']);
      expect(picker.value, 'everything', reason: 'the default');
      expect(key('agent-tool-set-hub'), findsNothing);
      expect(find.textContaining('The vault, plus files'), findsOneWidget);

      picker.onChanged!('read');
      await settle(t);
      await settleUntil(
          t, () => store.vaultDefaults.toolSetFor(AgentHost.local) == 'read');
      expect(
        await real(t, () => tv.vault.run((v) => v.agentToolSet(runner: 'this_device'))),
        'read',
      );
      expect(store.vaultDefaults.toolSetFor(AgentHost.hub), 'everything',
          reason: 'each runner has its own set');
      expect(
        t.widget<SelectButton<String>>(key('agent-tool-set-this_device')).value,
        'read',
      );

      // The hub is chosen: its set shows, with the restart note.
      await real(t, () => store.setVaultDefaultHost(AgentHost.hub));
      await settle(t);
      await settleUntil(t, () => key('agent-tool-set-hub').evaluate().isNotEmpty);
      expect(key('agent-tool-set-this_device'), findsNothing);
      await t.ensureVisible(key('agent-tool-set-hub'));
      picker = t.widget<SelectButton<String>>(key('agent-tool-set-hub'));
      expect(picker.value, 'everything', reason: 'the default');
      expect(find.textContaining('hub chats after the hub restarts'),
          findsOneWidget);

      picker.onChanged!('write');
      await settle(t);
      await settleUntil(
          t, () => store.vaultDefaults.toolSetFor(AgentHost.hub) == 'write');
      expect(
        await real(t, () => tv.vault.run((v) => v.agentToolSet(runner: 'hub'))),
        'write',
      );
      expect(store.vaultDefaults.toolSetFor(AgentHost.local), 'read',
          reason: 'the hub\'s pick leaves this computer\'s set alone');
    } finally {
      await closeAi(t, ai);
    }
  });
}
