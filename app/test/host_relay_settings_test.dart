// "Let the hub use this computer": one desktop toggle on Settings > Connectors >
// "This computer for hub apps", off by default, saved in this device's settings
// file only. Phones never show it. Without a hub sign-in the toggle is there
// but can't be turned on. While it is on, the computer's name on the hub, the
// relay's state and the connector address show below it.

import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart'
    show AgentHostRelayStatus, HostRelayLink;
import 'package:notes_app/features/settings/settings_kit.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/features/sync/hub_device_client.dart';

import 'harness.dart';

Finder key(String k) => find.byKey(ValueKey(k));

/// Signs in to a hub the relay cannot use (plain http), so turning the
/// toggle on never opens a socket in the test.
void _signIn(AppSettings s) => s.update(
  (s) => s
    ..clientUrl = 'http://hub.example.com'
    ..clientToken = 'test-token'
    ..clientDeviceLabel = 'Work PC',
);

/// Opens Settings > Connectors with "This computer for hub apps" open.
Future<void> _openHubPanel(WidgetTester tester, {bool mobile = false}) async {
  if (mobile) {
    await openMobileSettings(tester);
  } else {
    await tapAndSettle(tester, key('sidebar-settings'));
  }
  await tapAndSettle(tester, key('settings-nav-connectors'));
  await tester.ensureVisible(key('connectors-hub'));
  await tapAndSettle(tester, key('connectors-hub'));
  await settle(tester);
}

void main() {
  setUpAll(initCore);

  test('the setting is off by default and saved per device', () {
    final dir = Directory.systemTemp.createTempSync('host_relay_settings');
    try {
      final file = File('${dir.path}${Platform.pathSeparator}settings.json');
      final s = AppSettings.load(file);
      expect(s.hubUsesThisComputer, isFalse);
      s.update((s) => s.hubUsesThisComputer = true);
      expect(AppSettings.load(file).hubUsesThisComputer, isTrue);
      s.update((s) => s.hubUsesThisComputer = false);
      expect(AppSettings.load(file).hubUsesThisComputer, isFalse);
    } finally {
      dir.deleteSync(recursive: true);
    }
  });

  appTest('desktop: the toggle needs a hub sign-in, then turns on and off', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    final settings = tv.app.settings;
    await _openHubPanel(tester);
    // Not signed in: the toggle shows, disabled, with the reason.
    expect(find.text('Sign in to a hub first.'), findsOneWidget);
    expect(tester.widget<SSwitch>(key('host-relay-toggle')).onChanged, isNull);

    _signIn(settings);
    await settle(tester);
    expect(find.text('Let the hub use this computer'), findsOneWidget);
    expect(tester.widget<SSwitch>(key('host-relay-toggle')).value, isFalse);

    await tapAndSettle(tester, key('host-relay-toggle'));
    expect(tester.widget<SSwitch>(key('host-relay-toggle')).value, isTrue);
    expect(settings.hubUsesThisComputer, isTrue);

    await tapAndSettle(tester, key('host-relay-toggle'));
    expect(tester.widget<SSwitch>(key('host-relay-toggle')).value, isFalse);
    expect(settings.hubUsesThisComputer, isFalse);
  });

  appTest('desktop: while on, the hub name, the state and a copyable address', (
    tester,
  ) async {
    HostRelayLink.debugStatus = () async =>
        const AgentHostRelayStatus(state: 'connected');
    addTearDown(() => HostRelayLink.debugStatus = null);
    final copied = <String>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.setData') {
          copied.add((call.arguments as Map)['text'] as String);
        }
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
    final tv = await pumpApp(tester, mobile: false);
    final settings = tv.app.settings;
    _signIn(settings);
    await _openHubPanel(tester);
    expect(key('host-relay-name'), findsNothing, reason: 'off');

    await tapAndSettle(tester, key('host-relay-toggle'));
    expect(find.text('Appears on the hub as Work PC'), findsOneWidget);
    expect(
      find.text(
        'The name AI apps use for this computer. Renaming it changes the '
        'addresses below; the old ones stop working.',
      ),
      findsOneWidget,
    );
    // The connection row says it in words; its pill says "Connected".
    expect(find.text('Connected to the hub.'), findsOneWidget);
    expect(
      find.descendant(
        of: key('host-relay-status'),
        matching: find.text('Connected'),
      ),
      findsOneWidget,
    );
    const url = 'http://hub.example.com/mcp/agent?host=Work%20PC';
    expect(find.text('This computer only'), findsOneWidget);
    expect(tester.widget<Text>(key('host-relay-url-text')).data, url);
    expect(find.text('Any of your computers'), findsOneWidget);
    expect(
      tester.widget<Text>(key('host-relay-any-url-text')).data,
      'http://hub.example.com/mcp/agent',
    );

    final copy = find.descendant(
      of: key('host-relay-url'),
      matching: find.text('Copy'),
    );
    await tester.ensureVisible(copy);
    await tester.pumpAndSettle();
    await tapAndSettle(tester, copy);
    expect(copied, [url]);
    expect(
      find.descendant(of: key('host-relay-url'), matching: find.text('Copied')),
      findsOneWidget,
    );
    // Let the "Copied" label time out.
    await tester.pump(const Duration(seconds: 3));

    await tapAndSettle(tester, key('host-relay-toggle'));
    expect(key('host-relay-name'), findsNothing);
    expect(key('host-relay-any-url'), findsNothing);
  });

  appTest('desktop: Rename renames this computer on the hub', (tester) async {
    HostRelayLink.debugStatus = () async =>
        const AgentHostRelayStatus(state: 'connected');
    addTearDown(() => HostRelayLink.debugStatus = null);
    final asked = <String>[];
    debugRenameHubDevice =
        ({
          required String hubUrl,
          required String token,
          required String deviceId,
          required String name,
        }) async {
          asked.add(name);
          expect(hubUrl, 'http://hub.example.com');
          expect(token, 'test-token');
          if (name.toLowerCase() == 'laptop') {
            throw HubRenameException('Another computer is already called $name');
          }
          return name;
        };
    addTearDown(() => debugRenameHubDevice = null);
    final tv = await pumpApp(tester, mobile: false);
    final settings = tv.app.settings;
    _signIn(settings);
    await _openHubPanel(tester);
    await tapAndSettle(tester, key('host-relay-toggle'));
    final field = find.descendant(
      of: key('host-relay-rename-field'),
      matching: find.byType(EditableText),
    );

    // Another computer's name: an inline error, nothing saved.
    await tapAndSettle(tester, key('host-relay-rename'));
    expect(tester.widget<EditableText>(field).controller.text, 'Work PC');
    await tester.enterText(field, 'Laptop');
    await tapAndSettle(tester, key('host-relay-rename-save'));
    expect(
      find.text('Another computer is already called Laptop'),
      findsOneWidget,
    );
    expect(settings.clientDeviceLabel, 'Work PC');
    expect(
      tester.widget<Text>(key('host-relay-url-text')).data,
      'http://hub.example.com/mcp/agent?host=Work%20PC',
    );

    // A free name: saved, and the address follows it.
    await tester.enterText(field, '  Desk 2 ');
    await tapAndSettle(tester, key('host-relay-rename-save'));
    expect(asked, ['Laptop', 'Desk 2']);
    expect(settings.clientDeviceLabel, 'Desk 2');
    expect(key('host-relay-rename-field'), findsNothing);
    expect(key('host-relay-rename-error'), findsNothing);
    expect(find.text('Appears on the hub as Desk 2'), findsOneWidget);
    expect(
      tester.widget<Text>(key('host-relay-url-text')).data,
      'http://hub.example.com/mcp/agent?host=Desk%202',
    );
  });

  appTest('Android: phones never show the toggle', (tester) async {
    final tv = await pumpApp(tester, mobile: true);
    _signIn(tv.app.settings);
    await _openHubPanel(tester, mobile: true);
    expect(
      find.text('Set these from Scratch on a computer. Phones always use the hub.'),
      findsOneWidget,
    );
    expect(key('host-relay-toggle'), findsNothing);
  });
}
