@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'harness.dart';
import 'today_status_fixture.dart';

void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';
      appTest('Today status short cards $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark, empty: true);
        await seedTodayStatuses(tester);
        await tapAndSettle(tester, find.byKey(const ValueKey('today-overdue')));
        final more = find.byKey(const ValueKey('today-more'));
        while (more.evaluate().isNotEmpty) {
          await tester.ensureVisible(more);
          await tapAndSettle(tester, more);
        }
        await tester.ensureVisible(
          find.byKey(const ValueKey('today-selected-heading')),
        );
        await settle(tester);
        await expectLater(
          find.byType(Directionality).first,
          matchesGoldenFile('../screenshots/today-status-$tag.png'),
        );
      });
    }
  }
}
