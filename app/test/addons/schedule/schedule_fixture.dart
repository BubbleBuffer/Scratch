// Shared fixtures for the schedule add-on tests: seeded storage and backend,
// and a pump that wraps the schedule widgets in the app's theme and scopes.
// Goldens use the harness fonts and the fixed clock below.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/schedule/backend.dart';
import 'package:notes_app/addons/schedule/fake.dart';
import 'package:notes_app/addons/schedule/format.dart';
import 'package:notes_app/addons/schedule/storage.dart';
import 'package:notes_app/ui/tokens.dart';

import '../../harness.dart' show loadFonts;

const schedulePageId = 'sched-1';
const scheduleHubId = 'hub-1';
const scheduleLocalNodeId = 'desk-local';
const scheduleOtherNodeId = 'desk-other';

/// The device this client runs on, and another desktop on the same vault.
const scheduleLocalDeviceId = 'device-local';
const scheduleOtherDeviceId = 'device-other';

/// The template and parent pages the schedule points at, with real titles.
const scheduleTemplateId = 'tpl-1';
const scheduleParentId = 'journal';
const scheduleTemplateTitle = 'Weekly note';
const scheduleParentTitle = 'Journal';

/// Tue 6 Oct 2026, 11:20: the clock every schedule test and golden uses.
final scheduleNow = DateTime(2026, 10, 6, 11, 20);

/// A weekly schedule on Mondays at 08:00 (Berlin), run by the hub, with one
/// note step that makes a page from the "Weekly note" template.
Map<String, Object?> weeklyDefJson({
  String? runner = scheduleHubId,
  String? rrule = 'FREQ=WEEKLY;BYDAY=MO',
  String at = '2026-10-05T08:00',
  String tz = 'Europe/Berlin',
  String missed = 'latest',
}) => {
  'version': 1,
  'trigger': {'rrule': rrule, 'at': at, 'tz': tz, 'missed': missed},
  'runner': runner,
  'tools': ['create_page'],
  'steps': [
    <String, Object?>{
      'id': 'note',
      'do': 'note.from_template',
      'template': '[[$scheduleTemplateId]]',
      'parent': '[[$scheduleParentId]]',
      'title': 'Week {{week}} {{year}}',
    },
  ],
};

/// The runner's synced status: the last run created a page, the next run is
/// set.
Map<String, Object?> statusJson({bool withLastRun = true}) => {
  'runner_state': 'ok',
  'next_run': '2026-10-12T08:00',
  if (withLastRun)
    'last_run': {
      'occurrence': '2026-10-05T08:00',
      'at': '2026-10-05T08:00:04',
      'result': 'created',
      'page': 'pg-42',
    },
};

/// Storage holding one schedule page with the given definition and status,
/// and the real titles of its template and parent.
InMemoryScheduleStorage seededStorage({
  Map<String, Object?>? def,
  Map<String, Object?>? status,
}) => InMemoryScheduleStorage(
  seed: {
    schedulePageId: {
      scheduleAddonId: def ?? weeklyDefJson(),
      scheduleStatusKey: ?status,
    },
  },
  titles: const {
    scheduleTemplateId: scheduleTemplateTitle,
    scheduleParentId: scheduleParentTitle,
  },
);

/// Pumps [child] with the app's theme and the schedule scopes, at the phone
/// (375x812, 2x) or desktop (1440x900) size, in light or dark.
Future<void> pumpSchedule(
  WidgetTester tester,
  Widget child, {
  required ScheduleStorage storage,
  required ScheduleBackend backend,
  bool mobile = false,
  bool dark = false,
}) async {
  await tester.runAsync(loadFonts);
  sansFamily = 'Atkinson Hyperlegible Next';
  monoFamily = 'Fragment Mono';
  tester.view.devicePixelRatio = mobile ? 2.0 : 1.0;
  tester.view.physicalSize = mobile
      ? const Size(375 * 2, 812 * 2)
      : const Size(1440, 900);
  tester.platformDispatcher.platformBrightnessTestValue = dark
      ? Brightness.dark
      : Brightness.light;
  addTearDown(() {
    tester.view.reset();
    tester.platformDispatcher.clearAllTestValues();
  });
  final colors = dark ? AppColors.darkTheme : AppColors.light;
  await tester.pumpWidget(
    WidgetsApp(
      color: colors.bgPrimary,
      debugShowCheckedModeBanner: false,
      builder: (context, _) => AppTheme(
        colors: colors,
        formFactor: mobile ? FormFactor.mobile : FormFactor.desktop,
        child: ScheduleScope(
          storage: storage,
          child: ScheduleBackendScope(
            backend: backend,
            // MenuField's text fields need an Overlay above them, as in the app.
            child: Overlay(
              initialEntries: [OverlayEntry(builder: (_) => child)],
            ),
          ),
        ),
      ),
    ),
  );
  await settleSchedule(tester);
}

/// Lets the controller's storage and backend futures finish, then draws.
Future<void> settleSchedule(WidgetTester tester) async {
  for (var i = 0; i < 4; i++) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 5)),
    );
    await tester.pump(const Duration(milliseconds: 16));
  }
}

/// A hub, available and seen just now, plus the desktops given. The local
/// desktop is this client's device; the other one is another device.
FakeScheduleBackend hubBackend({
  Map<String, ScheduleLiveState>? states,
  bool hubAvailable = true,
  bool withDesktops = false,
  Object? error,
  Object? stateError,
}) => FakeScheduleBackend(
  nodes: [
    ScheduleNode(
      id: scheduleHubId,
      deviceId: 'device-hub',
      name: 'Home hub',
      kind: 'hub',
      available: hubAvailable,
      lastSeen: scheduleNow.subtract(const Duration(minutes: 2)),
    ),
    if (withDesktops) ...[
      ScheduleNode(
        id: scheduleLocalNodeId,
        deviceId: scheduleLocalDeviceId,
        name: 'This PC',
        kind: 'desktop',
        available: true,
        lastSeen: scheduleNow.subtract(const Duration(minutes: 1)),
      ),
      ScheduleNode(
        id: scheduleOtherNodeId,
        deviceId: scheduleOtherDeviceId,
        name: 'Laptop',
        kind: 'desktop',
        available: true,
        lastSeen: scheduleNow.subtract(const Duration(minutes: 3)),
      ),
    ],
  ],
  states: states,
  now: scheduleNow,
  error: error,
  stateError: stateError,
);

/// The live state of an accepted, running schedule with no runs yet.
const acceptedLive = ScheduleLiveState(
  accepted: true,
  hashMatches: true,
  paused: false,
  runnerState: 'ok',
);

/// The live state of a schedule nobody has accepted yet.
const unacceptedLive = ScheduleLiveState(
  accepted: false,
  hashMatches: false,
  paused: false,
  runnerState: 'needs_acceptance',
);

/// The live state of an accepted schedule that is paused.
const pausedLive = ScheduleLiveState(
  accepted: true,
  hashMatches: true,
  paused: true,
  runnerState: 'paused',
);

/// The live state of an accepted schedule whose runner is turned off.
const runnerOffLive = ScheduleLiveState(
  accepted: true,
  hashMatches: true,
  paused: false,
  runnerState: 'runner_off',
);
