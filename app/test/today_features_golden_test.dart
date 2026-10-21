@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'db_harness.dart' as d;
import 'harness.dart';
import 'today_editor_fixture.dart';

Future<void> shot(String name) => expectLater(find.byType(Directionality).first,
    matchesGoldenFile('../screenshots/today-$name.png'));

void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';
      appTest('to-do date chips $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        await openTodoDateFixture(tester);
        await shot('$tag-date-chips');
      });
      appTest('space colour menu $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        await openTodoDateFixture(tester);
        await tapAndSettle(tester, find.byKey(const ValueKey('page-menu')));
        await tapAndSettle(tester, find.byKey(const ValueKey('page-menu-colour')));
        await shot('$tag-colour-menu');
      });
      d.dbTest('Show in Today $tag', (tester) async {
        await d.pumpDb(tester, mobile: mobile, dark: dark);
        await d.openView(tester, 'Table', mobile: mobile);
        await d.tapKey(tester, 'db-settings');
        await d.tapKey(tester, 'settings-agenda');
        await shot('$tag-show-in-today');
      });
    }
  }
}
