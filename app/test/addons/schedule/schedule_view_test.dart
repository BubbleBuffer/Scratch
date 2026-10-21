// The schedule page and embed card: each state renders with its own actions,
// the status band sits above the editors, Accept passes the engine's hash of
// the definition on screen, another desktop is never called, the editors write
// only what they change, invalid input is shown and not stored, and the add-on
// registers its two slash items.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/schedule/api.dart';
import 'package:notes_app/addons/schedule/backend.dart';
import 'package:notes_app/addons/schedule/fake.dart';
import 'package:notes_app/addons/schedule/format.dart';
import 'package:notes_app/registry/addon_registry.dart';

import 'schedule_fixture.dart';

Future<void> pumpPage(
  WidgetTester tester, {
  required InMemoryScheduleStorage storage,
  required FakeScheduleBackend backend,
  bool mobile = false,
}) => pumpSchedule(
  tester,
  SchedulePage(
    pageId: schedulePageId,
    title: 'Weekly note schedule',
    now: () => scheduleNow,
    deviceTimeZone: 'UTC',
    localDeviceId: scheduleLocalDeviceId,
  ),
  storage: storage,
  backend: backend,
  mobile: mobile,
);

Finder _key(String name) => find.byKey(ValueKey(name));

void main() {
  group('status band', () {
    testWidgets('needs acceptance offers Accept and no run or pause', (
      tester,
    ) async {
      await pumpPage(
        tester,
        storage: seededStorage(status: statusJson(withLastRun: false)),
        backend: hubBackend(states: {schedulePageId: unacceptedLive}),
      );
      expect(_key('schedule-health-needsAcceptance'), findsOneWidget);
      expect(_key('schedule-accept'), findsOneWidget);
      expect(_key('schedule-pause'), findsNothing);
      expect(_key('schedule-run-now'), findsNothing);
      // The next run is the engine's preview: Monday 12 Oct at 08:00.
      expect(find.text('Mon 12 Oct, 08:00'), findsOneWidget);
      expect(find.text('Not run yet'), findsOneWidget);
    });

    testWidgets('running shows Pause and Run now, with the last run and a link', (
      tester,
    ) async {
      await pumpPage(
        tester,
        storage: seededStorage(status: statusJson()),
        backend: hubBackend(states: {schedulePageId: acceptedLive}),
      );
      expect(_key('schedule-health-ok'), findsOneWidget);
      expect(_key('schedule-pause'), findsOneWidget);
      expect(_key('schedule-run-now'), findsOneWidget);
      expect(_key('schedule-accept'), findsNothing);
      expect(find.textContaining('Created'), findsOneWidget);
      expect(_key('schedule-open-last'), findsOneWidget);
    });

    testWidgets('paused shows Resume in place of Pause', (tester) async {
      await pumpPage(
        tester,
        storage: seededStorage(status: statusJson()),
        backend: hubBackend(states: {schedulePageId: pausedLive}),
      );
      expect(_key('schedule-health-paused'), findsOneWidget);
      expect(find.text('Resume'), findsOneWidget);
    });

    testWidgets('an active schedule reads Active with a plain next run', (
      tester,
    ) async {
      await pumpPage(
        tester,
        storage: seededStorage(status: statusJson()),
        backend: hubBackend(states: {schedulePageId: acceptedLive}),
      );
      expect(find.text('Active'), findsOneWidget);
      expect(find.text('Next run'), findsOneWidget);
      expect(find.text('Next run (when active)'), findsNothing);
    });

    testWidgets('a schedule that is not running says its next run is conditional', (
      tester,
    ) async {
      await pumpPage(
        tester,
        storage: seededStorage(status: statusJson()),
        backend: hubBackend(states: {schedulePageId: pausedLive}),
      );
      expect(find.text('Next run (when active)'), findsOneWidget);
      expect(find.text('Next run'), findsNothing);
    });

    testWidgets('this computer runs its own schedule through its runner', (
      tester,
    ) async {
      final backend = hubBackend(
        withDesktops: true,
        states: {schedulePageId: acceptedLive},
      );
      await pumpPage(
        tester,
        storage: seededStorage(def: weeklyDefJson(runner: scheduleLocalNodeId)),
        backend: backend,
      );
      expect(_key('schedule-health-ok'), findsOneWidget);
      expect(
        backend.calls,
        contains('state:$schedulePageId:$scheduleLocalNodeId'),
      );
    });

    testWidgets('a runner that is turned off is named and offers no run', (
      tester,
    ) async {
      await pumpPage(
        tester,
        storage: seededStorage(status: statusJson()),
        backend: hubBackend(
          hubAvailable: false,
          states: {schedulePageId: runnerOffLive},
        ),
      );
      expect(_key('schedule-health-runnerOff'), findsOneWidget);
      expect(find.textContaining('runner turned off'), findsOneWidget);
      expect(_key('schedule-run-now'), findsNothing);
    });

    testWidgets('a definition from a newer format is refused', (tester) async {
      await pumpPage(
        tester,
        storage: seededStorage(def: weeklyDefJson()..['version'] = 2),
        backend: hubBackend(states: {schedulePageId: acceptedLive}),
      );
      expect(_key('schedule-health-refused'), findsOneWidget);
      expect(find.textContaining('needs a newer version'), findsOneWidget);
    });

    testWidgets('no runner says so and offers no action', (tester) async {
      await pumpPage(
        tester,
        storage: seededStorage(def: weeklyDefJson(runner: null)),
        backend: FakeScheduleBackend(nodes: const [], now: scheduleNow),
      );
      expect(_key('schedule-health-noRunner'), findsOneWidget);
      expect(_key('schedule-no-runner'), findsOneWidget);
      expect(_key('schedule-accept'), findsNothing);
      expect(_key('schedule-run-now'), findsNothing);
    });

    testWidgets('a hub that does not answer shows the synced status', (
      tester,
    ) async {
      await pumpPage(
        tester,
        storage: seededStorage(status: statusJson()),
        backend: hubBackend(
          stateError: const ScheduleUnavailable(
            'Hub not reachable: connection refused',
          ),
        ),
      );
      expect(_key('schedule-health-unreachable'), findsOneWidget);
      expect(find.text('Hub not reachable'), findsOneWidget);
      expect(
        find.text('Hub not reachable: connection refused'),
        findsOneWidget,
      );
      // The synced status still shows the last run and the next run.
      expect(find.textContaining('Created'), findsOneWidget);
      expect(_key('schedule-next-run'), findsOneWidget);
      expect(_key('schedule-accept'), findsNothing);
      expect(_key('schedule-run-now'), findsNothing);
    });

    testWidgets('another desktop is shown from the vault and never called', (
      tester,
    ) async {
      final backend = hubBackend(withDesktops: true);
      await pumpPage(
        tester,
        storage: seededStorage(
          def: weeklyDefJson(runner: scheduleOtherNodeId),
          status: statusJson(),
        ),
        backend: backend,
      );
      expect(_key('schedule-health-elsewhere'), findsOneWidget);
      expect(
        find.text('Open Scratch on Laptop to accept or pause'),
        findsOneWidget,
      );
      expect(_key('schedule-accept'), findsNothing);
      expect(_key('schedule-pause'), findsNothing);
      expect(_key('schedule-run-now'), findsNothing);
      expect(
        backend.calls.where((c) => c.contains(scheduleOtherNodeId)),
        isEmpty,
      );
    });

    testWidgets('the status band sits above When, which sits above Step', (
      tester,
    ) async {
      await pumpPage(
        tester,
        storage: seededStorage(status: statusJson()),
        backend: hubBackend(states: {schedulePageId: acceptedLive}),
      );
      final band = tester.getTopLeft(_key('schedule-status-band')).dy;
      final when = tester.getTopLeft(find.text('When')).dy;
      final runsOn = tester.getTopLeft(find.text('Runs on')).dy;
      final step = tester.getTopLeft(find.text('Step')).dy;
      expect(band, lessThan(when));
      expect(when, lessThan(runsOn));
      expect(runsOn, lessThan(step));
    });

    testWidgets('the primary action is above the fold on a phone', (
      tester,
    ) async {
      await pumpPage(
        tester,
        mobile: true,
        storage: seededStorage(status: statusJson(withLastRun: false)),
        backend: hubBackend(states: {schedulePageId: unacceptedLive}),
      );
      final accept = tester.getBottomLeft(_key('schedule-accept')).dy;
      expect(accept, lessThan(812));
    });
  });

  group('actions and editors', () {
    testWidgets('Accept passes the engine hash of the definition on screen', (
      tester,
    ) async {
      final backend = hubBackend(states: {schedulePageId: unacceptedLive});
      await pumpPage(
        tester,
        storage: seededStorage(status: statusJson(withLastRun: false)),
        backend: backend,
      );

      await tester.ensureVisible(_key('schedule-accept'));
      await tester.tap(_key('schedule-accept'));
      await settleSchedule(tester);

      final hash = fakeHash(
        ScheduleDef.fromJson(weeklyDefJson(), tz: 'UTC'),
      );
      expect(
        backend.calls,
        contains('accept:$schedulePageId:$scheduleHubId:$hash'),
      );
    });

    testWidgets('Run now asks the runner to run the schedule once', (
      tester,
    ) async {
      final backend = hubBackend(states: {schedulePageId: acceptedLive});
      await pumpPage(
        tester,
        storage: seededStorage(status: statusJson()),
        backend: backend,
      );

      await tester.ensureVisible(_key('schedule-run-now'));
      await tester.tap(_key('schedule-run-now'));
      await settleSchedule(tester);

      expect(backend.calls, contains('runNow:$schedulePageId:$scheduleHubId'));
    });

    testWidgets('the step shows the real template and parent titles', (
      tester,
    ) async {
      await pumpPage(
        tester,
        storage: seededStorage(status: statusJson()),
        backend: hubBackend(states: {schedulePageId: acceptedLive}),
      );
      expect(find.text(scheduleTemplateTitle), findsOneWidget);
      expect(find.text(scheduleParentTitle), findsOneWidget);
    });

    testWidgets('choosing Weekly writes FREQ=WEEKLY;BYDAY=MO and only that key', (
      tester,
    ) async {
      final storage = seededStorage(def: weeklyDefJson(rrule: null));
      await pumpPage(
        tester,
        storage: storage,
        backend: hubBackend(states: {schedulePageId: acceptedLive}),
      );

      await tester.tap(_key('schedule-freq-weekly'));
      await settleSchedule(tester);

      final trigger =
          storage.object(schedulePageId, scheduleAddonId)['trigger']
              as Map<String, Object?>;
      expect(trigger['rrule'], 'FREQ=WEEKLY;BYDAY=MO');
      expect(storage.writes, ['$schedulePageId/$scheduleAddonId/trigger']);
    });

    testWidgets('a weekday chip adds the day to BYDAY', (tester) async {
      final storage = seededStorage();
      await pumpPage(
        tester,
        storage: storage,
        backend: hubBackend(states: {schedulePageId: acceptedLive}),
      );

      await tester.tap(_key('schedule-day-FR'));
      await settleSchedule(tester);

      final trigger =
          storage.object(schedulePageId, scheduleAddonId)['trigger']
              as Map<String, Object?>;
      expect(trigger['rrule'], 'FREQ=WEEKLY;BYDAY=MO,FR');
    });

    testWidgets('an invalid time is named and not stored', (tester) async {
      final storage = seededStorage();
      await pumpPage(
        tester,
        storage: storage,
        backend: hubBackend(states: {schedulePageId: acceptedLive}),
      );

      final field = find.descendant(
        of: _key('schedule-time'),
        matching: find.byType(EditableText),
      );
      await tester.enterText(field, 'xx');
      await settleSchedule(tester);

      expect(find.text('Use 24-hour time, for example 08:00.'), findsOneWidget);
      expect(storage.writes, isEmpty);
      // The typed text stays, so the user can correct it.
      expect(tester.widget<EditableText>(field).controller.text, 'xx');
    });
  });

  group('embed card', () {
    testWidgets('shows the rule, the runner and the next run', (tester) async {
      await pumpSchedule(
        tester,
        const ScheduleEmbedCard(
          pageId: schedulePageId,
          title: 'Weekly note schedule',
          deviceTimeZone: 'UTC',
        ),
        storage: seededStorage(status: statusJson()),
        backend: hubBackend(states: {schedulePageId: acceptedLive}),
      );
      expect(find.textContaining('Monday'), findsOneWidget);
      expect(find.textContaining('Home hub'), findsOneWidget);
      expect(find.textContaining('Next:'), findsOneWidget);
    });
  });

  group('registration', () {
    test('the add-on offers both schedule slash items and block kinds', () {
      final ctx = AddonContext();
      bootAddons([const ScheduleAddon()], ctx);

      final plain = ctx.slashItems.of('schedule');
      expect(plain, isNotNull);
      expect(plain!.label, 'Schedule');
      expect(plain.blockKind, 'schedule');

      final weekly = ctx.slashItems.of('schedule_weekly');
      expect(weekly, isNotNull);
      expect(weekly!.label, 'Weekly note schedule');
      expect(weekly.blockKind, 'schedule_weekly');

      expect(ctx.blockKinds.of('schedule').insertOnly, isTrue);
      expect(ctx.blockKinds.of('schedule_weekly').insertOnly, isTrue);
    });
  });
}
