import 'dart:convert';
import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/reminders/reminder_plan.dart';
import 'package:notes_app/features/settings/controls.dart';
import 'package:notes_app/features/settings/settings_store.dart';

import 'harness.dart';

Finder key(String name) => find.byKey(ValueKey(name));

void main() {
  setUpAll(initCore);

  test('reminder defaults and preferences survive a settings reload', () {
    final dir = Directory.systemTemp.createTempSync('notes-reminder-settings-');
    try {
      final file = File('${dir.path}/settings.json');
      final settings = AppSettings.load(file);
      expect(settings.reminders, const ReminderSettings());
      settings.update(
        (s) => s.reminders = const ReminderSettings(
          enabled: false,
          leadMinutes: 30,
          morning: 390,
          permissionAsked: true,
        ),
      );
      expect(AppSettings.load(file).reminders, settings.reminders);
      expect(jsonDecode(file.readAsStringSync())['reminders'], {
        'enabled': false,
        'leadMinutes': 30,
        'morning': 390,
        'permissionAsked': true,
      });
    } finally {
      dir.deleteSync(recursive: true);
    }
  });

  test('invalid reminder values fall back independently to defaults', () {
    final dir = Directory.systemTemp.createTempSync('notes-reminder-settings-');
    try {
      final file = File('${dir.path}/settings.json');
      for (final morning in [-1, 1440, 315, '08:00']) {
        file.writeAsStringSync(
          jsonEncode({
            'reminders': {
              'enabled': false,
              'leadMinutes': 15,
              'morning': morning,
              'permissionAsked': true,
            },
          }),
        );
        expect(
          AppSettings.load(file).reminders,
          const ReminderSettings(enabled: false, permissionAsked: true),
        );
      }
    } finally {
      dir.deleteSync(recursive: true);
    }
  });

  for (final mobile in [false, true]) {
    appTest(
      'reminder controls update and disable on ${mobile ? 'mobile' : 'desktop'}',
      (tester) async {
        final tv = await pumpApp(tester, mobile: mobile);
        if (mobile) {
          await openMobileSettings(tester);
        } else {
          await tapAndSettle(tester, key('sidebar-settings'));
        }
        await tapAndSettle(tester, key('settings-nav-reminders'));
        expect(find.text('10 min'), findsOneWidget);
        expect(find.text('08:00'), findsOneWidget);

        await tapAndSettle(tester, key('reminders-lead'));
        await tapAndSettle(tester, find.text('At start'));
        expect(tv.app.settings.reminders.leadMinutes, 0);
        await tapAndSettle(tester, key('reminders-morning'));
        await tester.ensureVisible(find.text('09:30'));
        await tapAndSettle(tester, find.text('09:30'));
        expect(tv.app.settings.reminders.morning, 570);

        await tapAndSettle(tester, key('reminders-enabled'));
        expect(tv.app.settings.reminders.enabled, isFalse);
        expect(
          tester.widget<SelectButton<int>>(key('reminders-lead')).onChanged,
          isNull,
        );
        expect(
          tester.widget<SelectButton<int>>(key('reminders-morning')).onChanged,
          isNull,
        );
        await tapAndSettle(tester, key('reminders-lead'));
        expect(find.text('5 min'), findsNothing);
        await tapAndSettle(tester, key('reminders-morning'));
        expect(find.text('05:00'), findsNothing);
        await tapAndSettle(tester, key('reminders-enabled'));
        expect(
          tester.widget<SelectButton<int>>(key('reminders-lead')).onChanged,
          isNotNull,
        );
        expect(tv.app.settings.reminders.leadMinutes, 0);
        expect(tv.app.settings.reminders.morning, 570);
      },
    );
  }

  appTest('Android permission denial is a text-only note', (tester) async {
    final tv = await pumpApp(tester, mobile: true);
    tv.app.reminders.permissionGranted = false;
    await openMobileSettings(tester);
    await tapAndSettle(tester, key('settings-nav-reminders'));
    expect(
      find.text('Notifications are off for Scratch in Android settings.'),
      findsOneWidget,
    );
  });

  appTest('desktop does not show the Android permission note', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    tv.app.reminders.permissionGranted = false;
    await tapAndSettle(tester, key('sidebar-settings'));
    await tapAndSettle(tester, key('settings-nav-reminders'));
    expect(key('reminders-permission-note'), findsNothing);
  });
}
