// "Let the hub use this computer": one desktop toggle under the hub sign-in,
// off by default, saved in this device's settings file only. Phones never
// show it; neither does a device that is not signed in. While it is on, the
// computer's name on the hub, the relay's state and the connector address
// show below it.

import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart'
    show AgentHostRelayStatus, HostRelayLink;
import 'package:notes_app/features/settings/controls.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/features/sync/sync_settings.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';

Finder key(String k) => find.byKey(ValueKey(k));

class _HubPage extends StatelessWidget {
  const _HubPage();

  @override
  Widget build(BuildContext context) => ColoredBox(
    color: context.colors.bgPrimary,
    child: const SingleChildScrollView(
      padding: EdgeInsets.all(24),
      child: HubSignInSettings(),
    ),
  );
}

/// Signs in to a hub the relay cannot use (plain http), so turning the
/// toggle on never opens a socket in the test.
void _signIn(AppSettings s) => s.update(
  (s) => s
    ..clientUrl = 'http://hub.example.com'
    ..clientToken = 'test-token'
    ..clientDeviceLabel = 'Work PC',
);

Future<void> _openHubPage(WidgetTester tester) async {
  Navigator.of(
    tester.element(find.byType(HomeScreen)),
  ).push(PageRouteBuilder<void>(pageBuilder: (_, _, _) => const _HubPage()));
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

  appTest('desktop: the toggle shows when signed in and turns on and off', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    final settings = tv.app.settings;
    await _openHubPage(tester);
    expect(
      key('host-relay-toggle'),
      findsNothing,
      reason: 'not signed in to a hub',
    );

    _signIn(settings);
    await settle(tester);
    expect(find.text('Let the hub use this computer'), findsOneWidget);
    expect(tester.widget<Toggle>(key('host-relay-toggle')).value, isFalse);

    await tapAndSettle(tester, key('host-relay-toggle'));
    expect(tester.widget<Toggle>(key('host-relay-toggle')).value, isTrue);
    expect(settings.hubUsesThisComputer, isTrue);

    await tapAndSettle(tester, key('host-relay-toggle'));
    expect(tester.widget<Toggle>(key('host-relay-toggle')).value, isFalse);
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
    await _openHubPage(tester);
    expect(key('host-relay-name'), findsNothing, reason: 'off');

    await tapAndSettle(tester, key('host-relay-toggle'));
    expect(find.text('Appears on the hub as Work PC'), findsOneWidget);
    expect(
      find.text("This computer's name from when it signed in to the hub."),
      findsOneWidget,
    );
    expect(
      tester.widget<Text>(key('host-relay-status')).data,
      'Connected to the hub.',
    );
    const url = 'http://hub.example.com/mcp/agent?host=Work%20PC';
    expect(tester.widget<Text>(key('host-relay-url-text')).data, url);

    await tapAndSettle(
      tester,
      find.descendant(of: key('host-relay-url'), matching: find.text('Copy')),
    );
    expect(copied, [url]);
    expect(
      find.descendant(of: key('host-relay-url'), matching: find.text('Copied')),
      findsOneWidget,
    );
    // Let the "Copied" label time out.
    await tester.pump(const Duration(seconds: 3));

    await tapAndSettle(tester, key('host-relay-toggle'));
    expect(key('host-relay-name'), findsNothing);
  });

  appTest('Android: phones never show the toggle', (tester) async {
    final tv = await pumpApp(tester, mobile: true);
    _signIn(tv.app.settings);
    await _openHubPage(tester);
    expect(find.textContaining('Signed in to'), findsOneWidget);
    expect(key('host-relay-toggle'), findsNothing);
  });
}
