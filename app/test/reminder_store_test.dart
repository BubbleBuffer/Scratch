import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/core/debounce.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/ui/database/values.dart';
import 'package:notes_app/features/reminders/backends.dart';
import 'package:notes_app/features/reminders/reminder_plan.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/state/app_state.dart';

import 'harness.dart';

Future<void> drain(AppState app) async {
  await app.vault.flush();
  await app.feed.poll();
  // Feed debounce is still a real zero-delay timer under Debounce.instant.
  await Future<void>.delayed(Duration.zero);
  await app.reminders.replan();
}

Future<void> closeFixture(AppState app, Directory dir) async {
  app.feed.stop();
  await app.reminders.replan();
  var quiet = 0;
  final watch = Stopwatch()..start();
  while (quiet < 3 && watch.elapsed < const Duration(seconds: 10)) {
    await Future<void>.delayed(const Duration(milliseconds: 5));
    quiet = Vault.inFlight.value == 0 && Debounce.pending == 0 ? quiet + 1 : 0;
  }
  expect(quiet, 3, reason: 'fixture closes after all pending store reads');
  await app.dispose();
  await dir.delete(recursive: true);
}

Future<({Directory dir, AppState app, FakeReminderBackend fake, String calendar, String date})> fixture() async {
  resetTestClock();
  final dir = await Directory.systemTemp.createTemp('notes_reminders_');
  final vault = await Vault.open('${dir.path}/vault');
  final calendar = await vault.createPage(title: 'Calendar');
  await vault.run((v) => v.makeCollection(page: calendar), writes: true);
  final schema = await vault.run((v) => v.setSchema(collection: calendar, schema: [
    const PropDef(key: '', name: 'When', kind: PropKind.date, options: [], config: ''),
  ]), writes: true);
  final settings = AppSettings();
  final fake = FakeReminderBackend(settings: settings);
  final app = AppState(vault, settings: settings, reminderBackend: fake);
  await app.start();
  return (dir: dir, app: app, fake: fake, calendar: calendar, date: schema.single.key);
}

Future<String> addEvent(AppState app, String calendar, String date) async {
  final row = await app.vault.createPage(parent: calendar, title: 'Meeting');
  await app.vault.run((v) => v.setProperty(page: row, key: date,
    value: Values.date(DateTime(2026, 10, 6, 14).millisecondsSinceEpoch, hasTime: true)), writes: true);
  return row;
}

void main() {
  setUpAll(initCore);
  tearDown(() => Clock.fixedAt = null);

  test('start applies and a Calendar event replans through the debounce', () async {
    final f = await fixture();
    try {
      expect(f.fake.appliedPlans, hasLength(1));
      final row = await addEvent(f.app, f.calendar, f.date);
      await f.app.vault.flush();
      await f.app.feed.poll();
      final watch = Stopwatch()..start();
      while (!f.app.reminders.plan.any((r) => r.title == 'Meeting') && watch.elapsed < const Duration(seconds: 10)) {
        await Future<void>.delayed(const Duration(milliseconds: 5));
      }
      expect(f.app.reminders.plan.singleWhere((r) => r.kind == ReminderKind.event).payload,
        jsonEncode({'page': row, 'block': null}));
      expect(f.fake.appliedPlans.length, greaterThan(1));
    } finally { await closeFixture(f.app, f.dir); }
  });

  test('lead changes, unchanged plans, once-only permission and disabling', () async {
    final f = await fixture();
    try {
      await addEvent(f.app, f.calendar, f.date);
      await drain(f.app);
      final initial = f.app.reminders.plan.singleWhere((r) => r.kind == ReminderKind.event);
      final calls = f.fake.appliedPlans.length;
      await f.app.reminders.replan();
      expect(f.fake.appliedPlans.length, calls);
      f.app.settings.update((s) => s.reminders = s.reminders.copyWith(leadMinutes: 30));
      await f.app.reminders.replan();
      final shifted = f.app.reminders.plan.singleWhere((r) => r.kind == ReminderKind.event);
      expect(shifted.instant, initial.instant.subtract(const Duration(minutes: 20)));
      expect(f.fake.permissionRequests, 1);
      expect(f.app.settings.reminders.permissionAsked, isTrue);
      f.app.settings.update((s) => s.reminders = s.reminders.copyWith(enabled: false));
      await f.app.reminders.replan();
      expect(f.fake.cancelCalls, 1);
      expect(f.app.reminders.plan, isEmpty);
      await f.app.reminders.replan();
      expect(f.fake.cancelCalls, 1);
    } finally { await closeFixture(f.app, f.dir); }
  });

  test('ticking a timed task removes its reminder and summary', () async {
    final f = await fixture();
    try {
      final schema = await f.app.vault.run((v) => v.getSchema(collection: f.calendar));
      await f.app.vault.run((v) => v.setSchema(collection: f.calendar, schema: [
        ...schema,
        const PropDef(key: '', name: 'Done', kind: PropKind.checkbox, options: [], config: ''),
      ]), writes: true);
      await f.app.vault.run((v) => v.setAgendaSettings(collection: f.calendar,
        settings: const AgendaSettings(role: AgendaRole.tasks)), writes: true);
      final row = await addEvent(f.app, f.calendar, f.date);
      await drain(f.app);
      expect(f.app.reminders.plan.where((r) => r.kind == ReminderKind.task), hasLength(1));
      expect(f.app.reminders.plan.first.body, 'Due now · Calendar');
      await f.app.vault.run((v) => v.setAgendaDone(page: row, done: true), writes: true);
      await drain(f.app);
      expect(f.app.reminders.plan, isEmpty);
    } finally { await closeFixture(f.app, f.dir); }
  });

  appTest('fake tap opens its page and block, and today returns Home', (tester) async {
    final tv = await pumpApp(tester, mobile: false, demo: true);
    final item = tv.app.agenda.items.firstWhere((i) => i.block != null && i.title == 'Plant garlic');
    final fake = tv.app.reminders.backend as FakeReminderBackend;
    fake.emitTap(jsonEncode({'page': item.page, 'block': item.block}));
    await settle(tester);
    expect(find.byType(PageScreen), findsOneWidget);
    expect(tester.widget<PageScreen>(find.byType(PageScreen)).initialBlock, item.block);
    expect(tv.app.ui.lastPage, item.page);
    fake.emitTap('today');
    await settle(tester);
    expect(find.byType(PageScreen), findsNothing);
    expect(tv.app.agenda.isToday, isTrue);
  });
}
