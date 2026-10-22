// "Agent runs on": the host choice from the agent setting and the sync
// settings' hub (an https:// hub with a device token), and the setting's
// round trip through ai_settings.json. No bridge needed.

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/settings/settings_store.dart';

AppSettings syncsWith(String? url, String? token) => AppSettings()
  ..clientUrl = url
  ..clientToken = token;

void main() {
  test('the hub counts only as an https:// hub with a device token', () {
    final link = HubLink.of(syncsWith(' https://notes.example.com ', 'tok'));
    expect(link?.url, 'https://notes.example.com');
    expect(link?.token, 'tok');
    expect(HubLink.of(syncsWith('HTTPS://notes.example.com', 'tok')), isNotNull);
    expect(HubLink.of(syncsWith('http://192.168.0.5:47100', 'tok')), isNull);
    expect(HubLink.of(syncsWith('https://notes.example.com', '')), isNull);
    expect(HubLink.of(syncsWith('https://notes.example.com', null)), isNull);
    expect(HubLink.of(syncsWith(null, null)), isNull);
    expect(HubLink.of(null), isNull);
  });

  test('the agent runs on the hub only when chosen and the hub is usable', () {
    final hub = HubLink.of(syncsWith('https://notes.example.com', 'tok'));
    final both = [AgentHost.local, if (hub != null) AgentHost.hub];
    final local = [AgentHost.local];
    ResolvedAgentHost pick({AgentHost? device, AgentHost? vault, required List<AgentHost> usable}) =>
        resolveAgentHost(
          device: device,
          vault: vault,
          builtIn: AgentHost.local,
          usable: usable,
        );
    expect(pick(usable: both).host, AgentHost.local, reason: 'the default');
    expect(pick(device: AgentHost.hub, usable: both).host, AgentHost.hub);
    expect(pick(vault: AgentHost.hub, usable: both).host, AgentHost.hub,
        reason: 'the vault default applies without a device override');
    expect(pick(device: AgentHost.local, vault: AgentHost.hub, usable: both).host,
        AgentHost.local, reason: 'the device override wins over the vault');
    final none = pick(device: AgentHost.hub, usable: local);
    expect(none.host, AgentHost.local, reason: 'no usable hub: this computer');
    expect(none.wanted, AgentHost.hub);
    expect(none.fellBack, isTrue);
  });

  test('an old agentHostOverride in ai_settings.json loads and is not saved', () {
    final dir = Directory.systemTemp.createTempSync('agent_host_test');
    addTearDown(() => dir.deleteSync(recursive: true));
    final file = File('${dir.path}${Platform.pathSeparator}ai_settings.json');
    file.writeAsStringSync('{"agentHostOverride": "hub", "agentHost": "moon"}');
    expect(() => AiSettings.load(file), returnsNormally);
    expect(AiSettings.load(file).legacyAgentHost, isNull,
        reason: 'an unknown runner is no default');
    AiSettings.load(file).update((s) => s.agentEnabled = true);
    final saved = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
    expect(saved.containsKey('agentHostOverride'), isFalse);
  });

  test('computer access defaults to home and round-trips with its root', () {
    final dir = Directory.systemTemp.createTempSync('agent_computer_settings');
    addTearDown(() => dir.deleteSync(recursive: true));
    final file = File('${dir.path}${Platform.pathSeparator}ai_settings.json');
    final defaults = AiSettings.load(file);
    expect(defaults.desktopTools, isTrue);
    expect(defaults.desktopRoot, Platform.environment[Platform.isWindows ? 'USERPROFILE' : 'HOME'] ?? '');
    defaults.update((s) {
      s.desktopTools = false;
      s.desktopRoot = dir.path;
    });
    final loaded = AiSettings.load(file);
    expect(loaded.desktopTools, isFalse);
    expect(loaded.desktopRoot, dir.path);
    expect(loaded.toJson()['desktopTools'], isFalse);
    expect(loaded.toJson()['desktopRoot'], dir.path);
  });
}
