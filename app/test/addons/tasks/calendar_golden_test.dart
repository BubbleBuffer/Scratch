import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/api.dart';

import '../../addon_harness.dart';
import 'calendar_test.dart' show calendarContext, seedCalendarTask;

// Source-only goldens. Baseline PNGs must be reviewed in the normal golden
// workflow; this feature does not create or update any image baseline.
void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      for (final period in ['Month', 'Week', 'Day']) {
        appTest(
          'Calendar $period ${mobile ? 'phone' : 'desktop'} ${dark ? 'dark' : 'light'}',
          (tester) async {
            final tv = await pumpApp(
              tester,
              mobile: mobile,
              dark: dark,
              empty: true,
            );
            final dpr = mobile ? 2.0 : 1.0;
            tester.view.physicalSize = Size(
              (mobile ? 390 : 1280) * dpr,
              (mobile ? 844 : 800) * dpr,
            );
            await seedCalendarTask(
              tester,
              tv.vault,
              'Plan the week',
              DateTime(2026, 10, 6),
            );
            await seedCalendarTask(
              tester,
              tv.vault,
              'Design review',
              DateTime(2026, 10, 6, 10),
              timed: true,
              duration: 60,
            );
            await seedCalendarTask(
              tester,
              tv.vault,
              'Read and collect references',
              DateTime(2026, 10, 7),
            );
            unawaited(openCalendar(calendarContext(tester)));
            await settle(tester);
            if (period != 'Month') {
              await tester.tap(find.text(period));
              await settle(tester);
            }
            expect(tester.takeException(), isNull);
            await expectLater(
              find.byType(VaultCalendarScreen),
              matchesGoldenFile(
                'goldens/calendar-${period.toLowerCase()}-${mobile ? 'phone' : 'desktop'}-${dark ? 'dark' : 'light'}.png',
              ),
            );
          },
          tags: ['golden'],
        );
      }
    }
  }
}
