// Golden screenshots of the schedule: the page in four states (needs
// acceptance, active with a last run, runner turned off, this computer as the
// runner) and the compact embed card, at 1440x900 (desktop) and 375x812
// (phone), light and dark. Files: screenshots/schedule-<desktop|phone>-<light|dark>-<name>.png.
// Regenerate with: flutter test --update-goldens test/addons/schedule/schedule_golden_test.dart

@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/schedule/embed.dart';
import 'package:notes_app/addons/schedule/view.dart';
import 'package:notes_app/ui/tokens.dart';

import 'schedule_fixture.dart';

SchedulePage _page() => SchedulePage(
  pageId: schedulePageId,
  title: 'Weekly note schedule',
  now: () => scheduleNow,
  deviceTimeZone: 'UTC',
  localDeviceId: scheduleLocalDeviceId,
);

void main() {
  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final form = mobile ? 'phone' : 'desktop';
      final theme = dark ? 'dark' : 'light';

      testWidgets('schedule $form $theme needs acceptance', (tester) async {
        await pumpSchedule(
          tester,
          _page(),
          storage: seededStorage(status: statusJson(withLastRun: false)),
          backend: hubBackend(states: {schedulePageId: unacceptedLive}),
          mobile: mobile,
          dark: dark,
        );
        await expectLater(
          find.byType(WidgetsApp),
          matchesGoldenFile(
            '../../../screenshots/schedule-$form-$theme-needs-acceptance.png',
          ),
        );
      }, tags: ['golden']);

      testWidgets('schedule $form $theme active with last run', (tester) async {
        await pumpSchedule(
          tester,
          _page(),
          storage: seededStorage(status: statusJson()),
          backend: hubBackend(states: {schedulePageId: acceptedLive}),
          mobile: mobile,
          dark: dark,
        );
        await expectLater(
          find.byType(WidgetsApp),
          matchesGoldenFile(
            '../../../screenshots/schedule-$form-$theme-active-last-run.png',
          ),
        );
      }, tags: ['golden']);

      testWidgets('schedule $form $theme runner off', (tester) async {
        await pumpSchedule(
          tester,
          _page(),
          storage: seededStorage(status: statusJson()),
          backend: hubBackend(
            hubAvailable: false,
            states: {schedulePageId: runnerOffLive},
          ),
          mobile: mobile,
          dark: dark,
        );
        await expectLater(
          find.byType(WidgetsApp),
          matchesGoldenFile(
            '../../../screenshots/schedule-$form-$theme-runner-off.png',
          ),
        );
      }, tags: ['golden']);

      testWidgets('schedule $form $theme local desktop runner', (tester) async {
        await pumpSchedule(
          tester,
          _page(),
          storage: seededStorage(
            def: weeklyDefJson(runner: scheduleLocalNodeId),
            status: statusJson(),
          ),
          backend: hubBackend(
            withDesktops: true,
            states: {schedulePageId: acceptedLive},
          ),
          mobile: mobile,
          dark: dark,
        );
        await expectLater(
          find.byType(WidgetsApp),
          matchesGoldenFile(
            '../../../screenshots/schedule-$form-$theme-local-runner.png',
          ),
        );
      }, tags: ['golden']);

      testWidgets('schedule $form $theme embed card', (tester) async {
        final colors = dark ? AppColors.darkTheme : AppColors.light;
        await pumpSchedule(
          tester,
          // The root takes the whole screen; Align lets the card keep its own
          // height, as a block in a note does.
          SizedBox.expand(
            child: ColoredBox(
              color: colors.bgPrimary,
              child: Align(
                alignment: Alignment.topLeft,
                child: Padding(
                  padding: const EdgeInsets.all(16),
                  child: const ScheduleEmbedCard(
                    pageId: schedulePageId,
                    title: 'Weekly note schedule',
                    deviceTimeZone: 'UTC',
                    localDeviceId: scheduleLocalDeviceId,
                  ),
                ),
              ),
            ),
          ),
          storage: seededStorage(status: statusJson()),
          backend: hubBackend(states: {schedulePageId: acceptedLive}),
          mobile: mobile,
          dark: dark,
        );
        await expectLater(
          find.byType(WidgetsApp),
          matchesGoldenFile(
            '../../../screenshots/schedule-$form-$theme-embed.png',
          ),
        );
      }, tags: ['golden']);
    }
  }
}
