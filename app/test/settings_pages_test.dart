// The settings pages that follow the approved redesign: General, Sync and
// devices, Conflicts and Storage. Each opens from the settings rail and shows
// its title, the controls the design names, and collapsed advanced panels.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/settings/settings_kit.dart';
import 'package:notes_app/features/settings/settings_store.dart';

import 'harness.dart';

Finder key(String name) => find.byKey(ValueKey(name));

Future<void> openSection(WidgetTester tester, String name) async {
  await tapAndSettle(tester, key('sidebar-settings'));
  await tapAndSettle(tester, key('settings-nav-$name'));
}

void main() {
  setUpAll(initCore);

  appTest('General: segmented theme and a Write feedback button', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    await openSection(tester, 'general');
    expect(find.text('General'), findsWidgets);
    expect(find.text('Preferences'), findsNothing);
    expect(key('theme-system'), findsOneWidget);
    expect(key('theme-light'), findsOneWidget);
    expect(key('theme-dark'), findsOneWidget);
    expect(find.text('Write feedback'), findsOneWidget);
    expect(key('send-feedback'), findsOneWidget);
    await tapAndSettle(tester, key('theme-light'));
    expect(tv.app.settings.theme, ThemePref.light);
  });

  appTest('Sync and devices: status card, headings, pairing collapsed', (tester) async {
    await pumpApp(tester, mobile: false);
    await openSection(tester, 'sync');
    expect(find.text('Sync and devices'), findsWidgets);
    expect(find.text('Not syncing'), findsOneWidget);
    expect(find.text('Your hub'), findsOneWidget);
    expect(find.text('Attachments'), findsOneWidget);
    expect(find.text('This computer as a hub'), findsOneWidget);
    expect(find.text('Connect with a pairing code'), findsOneWidget);
    expect(key('connect-hub-field'), findsNothing, reason: 'the pairing field starts collapsed');
    await tester.ensureVisible(key('connect-advanced'));
    await tapAndSettle(tester, key('connect-advanced'));
    expect(key('connect-hub-field'), findsOneWidget);
  });

  appTest('Conflicts: title and the empty state', (tester) async {
    await pumpApp(tester, mobile: false);
    await openSection(tester, 'conflicts');
    expect(find.text('Conflicts'), findsWidgets);
    expect(find.text('Changed on two devices'), findsNothing);
    expect(key('no-conflicts'), findsOneWidget);
  });

  appTest('Storage: location shown, Open another folder collapsed, mounted folders empty', (tester) async {
    await pumpApp(tester, mobile: false);
    await openSection(tester, 'storage');
    expect(find.text('Storage'), findsWidgets);
    expect(key('vault-path'), findsOneWidget);
    expect(key('vault-path-field'), findsNothing, reason: 'Open another folder starts collapsed');
    await tester.ensureVisible(key('vault-open-another'));
    await tapAndSettle(tester, key('vault-open-another'));
    expect(key('vault-path-field'), findsOneWidget);
    expect(find.text('No mounted folders'), findsOneWidget);
    expect(key('mount-open'), findsOneWidget);
  });

  appTest('Reminders and About use the kit', (tester) async {
    await pumpApp(tester, mobile: false);
    await openSection(tester, 'reminders');
    expect(find.byType(SSwitch), findsWidgets);
    expect(find.byType(SSelect<int>), findsNWidgets(2));
    await tapAndSettle(tester, key('settings-nav-about'));
    expect(find.text('Version'), findsWidgets);
    expect(key('device-id'), findsOneWidget);
  });
}
