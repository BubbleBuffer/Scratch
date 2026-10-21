// "Agent runs on": the host choice from the agent setting and the sync
// settings' hub (an https:// hub with a device token), and the setting's
// round trip through ai_settings.json. No bridge needed.

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
    final here = AiSettings();
    final there = AiSettings()..agentHost = AgentHost.hub;
    expect(here.agentHost, AgentHost.local, reason: 'the default');
    expect(agentHostFor(here, hub), AgentHost.local);
    expect(agentHostFor(there, hub), AgentHost.hub);
    expect(agentHostFor(there, null), AgentHost.local, reason: 'no usable hub: this computer');
  });

  test('the host setting is saved in ai_settings.json', () {
    final dir = Directory.systemTemp.createTempSync('agent_host_test');
    addTearDown(() => dir.deleteSync(recursive: true));
    final file = File('${dir.path}${Platform.pathSeparator}ai_settings.json');
    AiSettings.load(file).update((s) => s.agentHost = AgentHost.hub);
    expect(AiSettings.load(file).agentHost, AgentHost.hub);
    file.writeAsStringSync('{"agentHost": "moon"}');
    expect(AiSettings.load(file).agentHost, AgentHost.local);
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
