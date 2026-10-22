// Integration spec phase 3: the default runner and default model are vault
// settings (synced on the hidden settings page). An unreachable runner falls
// back to a reachable one. Old per-device values go into the vault once, only
// when it has none, and are not kept as overrides. Per-device overrides are
// no longer applied: one left in ai_settings.json by an older build is ignored.

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/features/ai/ai_settings.dart';

import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);

  group('precedence', () {
    test('device override, then vault, then built-in', () {
      const both = [AgentHost.local, AgentHost.hub];
      var r = resolveAgentHost(usable: both, builtIn: AgentHost.local);
      expect((r.host, r.source), (AgentHost.local, AgentDefaultSource.builtIn));
      r = resolveAgentHost(
        vault: AgentHost.hub,
        usable: both,
        builtIn: AgentHost.local,
      );
      expect((r.host, r.source), (AgentHost.hub, AgentDefaultSource.vault));
      r = resolveAgentHost(
        device: AgentHost.local,
        vault: AgentHost.hub,
        usable: both,
        builtIn: AgentHost.local,
      );
      expect((r.host, r.source), (AgentHost.local, AgentDefaultSource.device));
      expect(r.fellBack, isFalse);
    });

    test('an unreachable runner falls back and says so', () {
      final desktop = resolveAgentHost(
        vault: AgentHost.hub,
        usable: const [AgentHost.local],
        builtIn: AgentHost.local,
      );
      expect(desktop.host, AgentHost.local);
      expect(desktop.wanted, AgentHost.hub);
      expect(desktop.fellBack, isTrue);
      final android = resolveAgentHost(
        vault: AgentHost.local,
        usable: const [AgentHost.hub],
        builtIn: AgentHost.hub,
      );
      expect(android.host, AgentHost.hub, reason: 'Android goes to the Hub');
      expect(android.fellBack, isTrue);
    });

    test('model: device override, then vault, per runner', () {
      const vault = {AgentHost.hub: 'codex/hub', AgentHost.local: 'codex/here'};
      expect(resolveAgentModel(AgentHost.hub, vault: vault).model, 'codex/hub');
      final o = resolveAgentModel(
        AgentHost.hub,
        device: const {AgentHost.hub: 'codex/mine'},
        vault: vault,
      );
      expect((o.model, o.source), ('codex/mine', AgentDefaultSource.device));
      expect(
        resolveAgentModel(
          AgentHost.local,
          device: const {AgentHost.hub: 'codex/mine'},
          vault: vault,
        ).model,
        'codex/here',
        reason: 'an override is per runner',
      );
      expect(resolveAgentModel(AgentHost.hub).model, isNull);
    });
  });

  test('overrides round-trip; legacy keys are read but not re-saved once cleared', () {
    final dir = Directory.systemTemp.createTempSync('agent_defaults_');
    addTearDown(() => dir.deleteSync(recursive: true));
    final file = File('${dir.path}/ai_settings.json')
      ..writeAsStringSync('{"agentHost":"hub","agentModel":"codex/old"}');
    final s = AiSettings.load(file);
    expect(s.legacyAgentHost, AgentHost.hub);
    expect(s.legacyAgentModel, 'codex/old');
    expect(s.agentModelOverrides, isEmpty);
    s.update((s) {
      s.legacyAgentHost = null;
      s.legacyAgentModel = null;
      s.agentModelOverrides[AgentHost.hub] = 'codex/mine';
    });
    final saved = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
    expect(saved.containsKey('agentHost'), isFalse);
    expect(saved.containsKey('agentModel'), isFalse);
    final back = AiSettings.load(file);
    expect(back.agentModelOverrides, {AgentHost.hub: 'codex/mine'});
    expect(back.hasLegacyAgentDefaults, isFalse);
  });

  appTest('the vault default syncs; a device override from an older build is ignored', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final file = File('${tv.dir.path}/synced-defaults-settings.json');
      final settings = AiSettings.load(file);
      final store = AgentStore.attach(tv.vault, settings);
      await real(tester, store.refreshVaultDefaults);
      expect(store.vaultDefaults, VaultAgentDefaultsView.none);

      // Another device sets the vault value; it reaches this one.
      await real(tester, () => other.vault.run((v) => v.setAgentDefaultModel(runner: 'this_device', model: 'codex/vault-pick')));
      await real(tester, () => other.vault.run((v) => v.setAgentDefaultRunner(runner: 'this_device')));
      expect(await real(tester, () => other.client!.syncOnce()), isNotNull);
      await real(tester, store.refreshVaultDefaults);
      expect(store.vaultDefaults.host, AgentHost.local);
      expect(store.vaultDefaults.models[AgentHost.local], 'codex/vault-pick');
      expect(store.resolvedModel(AgentHost.local).model, 'codex/vault-pick');

      // A device override left in ai_settings.json is not applied: the
      // vault value wins, and the vault is not changed.
      store.settings.update((s) => s.agentModelOverrides[AgentHost.local] = 'codex/mine');
      expect(store.resolvedModel(AgentHost.local).model, 'codex/vault-pick');
      expect(store.resolvedModel(AgentHost.local).source, AgentDefaultSource.vault);
      final d = await real(tester, () => tv.vault.run((v) => v.agentDefaults()));
      expect(d.models.single.model, 'codex/vault-pick');

      // This device writes the vault value; the other device gets it.
      await real(tester, () => store.setVaultDefaultHost(AgentHost.hub));
      await real(tester, () => store.setVaultDefaultModel(AgentHost.hub, 'codex/hub-pick'));
      expect(store.vaultDefaults.host, AgentHost.hub);
      expect(store.resolvedHost.wanted, AgentHost.hub);
      expect(store.resolvedHost.fellBack, isTrue, reason: 'no https hub here: this computer');
      expect(store.defaultHost, AgentHost.local);
      expect(await real(tester, () => other.client!.syncOnce()), isNotNull);
      final there = await real(tester, () => other.vault.run((v) => v.agentDefaults()));
      expect(there.runner, 'hub');
      expect({for (final m in there.models) m.runner: m.model}, {'this_device': 'codex/vault-pick', 'hub': 'codex/hub-pick'});

      // An agentHostOverride left in an old ai_settings.json loads without
      // error and is not applied: the vault's runner (the hub, unreachable
      // here) still decides.
      final old = File('${tv.dir.path}/old-override-settings.json')
        ..writeAsStringSync('{"agentHostOverride": "local"}');
      final oldStore = AgentStore.attach(tv.vault, AiSettings.load(old));
      await real(tester, oldStore.refreshVaultDefaults);
      expect(oldStore.resolvedHost.source, AgentDefaultSource.vault);
      expect(oldStore.resolvedHost.wanted, AgentHost.hub);
      expect(oldStore.resolvedHost.fellBack, isTrue);
    } finally {
      await other.close(tester);
    }
  });

  appTest('old per-device values seed an empty vault once and are dropped', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final file = File('${tv.dir.path}/legacy-settings.json')
      ..writeAsStringSync('{"agentHost":"local","agentModel":"codex/legacy"}');
    // Created in real async: the migration it starts writes to the vault.
    final store = await real(tester, () async => AgentStore.attach(tv.vault, AiSettings.load(file)));
    await real(tester, store.refreshVaultDefaults);
    final d = await real(tester, () => tv.vault.run((v) => v.agentDefaults()));
    expect(d.runner, 'this_device');
    expect({for (final m in d.models) m.runner: m.model}, {'this_device': 'codex/legacy'});
    final saved = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
    expect(saved.containsKey('agentModel'), isFalse, reason: 'not kept as an override');
    expect(store.settings.agentModelOverrides, isEmpty);

    // A second device's old values don't replace the vault's.
    final second = File('${tv.dir.path}/legacy-settings-2.json')
      ..writeAsStringSync('{"agentHost":"hub","agentModel":"codex/other"}');
    await real(tester, () async {
      store.useSettings(AiSettings.load(second));
      await store.refreshVaultDefaults();
    });
    final again = await real(tester, () => tv.vault.run((v) => v.agentDefaults()));
    expect(again.runner, 'this_device');
    expect(again.models.single.model, 'codex/legacy');
    expect(store.settings.hasLegacyAgentDefaults, isFalse);
  });
}
