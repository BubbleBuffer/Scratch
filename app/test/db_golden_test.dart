// Golden screenshots of databases: every view, the property menu, the
// filter and sort editors, a row page with properties (a sheet with its
// database in the breadcrumb on desktop), the
// view switcher and an inline database, at 1440x900 (desktop) and 375x812
// (mobile), light and dark. Files: screenshots/db-<form>-<theme>-<scene>.png.
// Regenerate with: flutter test --update-goldens test/db_golden_test.dart

@Tags(['golden'])
library;

import 'package:flutter_test/flutter_test.dart';

import 'db_harness.dart';

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = 'db-${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';

      for (final view in ['Table', 'Board', 'Calendar', 'Timeline', 'List', 'Gallery']) {
        dbTest('$view view $tag', (tester) async {
          await pumpDb(tester, mobile: mobile, dark: dark);
          await openView(tester, view, mobile: mobile);
          await shot('$tag-${view.toLowerCase()}');
        });
      }

      dbTest('property menu $tag', (tester) async {
        await pumpDb(tester, mobile: mobile, dark: dark);
        await openView(tester, 'Table', mobile: mobile);
        await tapKey(tester, 'col-Status');
        await shot('$tag-property-menu');
      });

      dbTest('property type picker $tag', (tester) async {
        await pumpDb(tester, mobile: mobile, dark: dark);
        await openView(tester, 'Table', mobile: mobile);
        await tapKey(tester, mobile ? 'col-Status' : 'col-Priority');
        await tapKey(tester, 'prop-edit');
        await shot('$tag-edit-property');
      });

      dbTest('filter editor $tag', (tester) async {
        await pumpDb(tester, mobile: mobile, dark: dark);
        await openView(tester, 'Table', mobile: mobile);
        await tapKey(tester, 'db-filter');
        await tapKey(tester, 'pick-Status');
        await shot('$tag-filter');
      });

      dbTest('advanced filter $tag', (tester) async {
        await pumpDb(tester, mobile: mobile, dark: dark);
        await openView(tester, 'Table', mobile: mobile);
        await tapKey(tester, 'db-filter');
        await tapKey(tester, 'pick-Status');
        await tapKey(tester, 'rule-more');
        await tapKey(tester, 'rule-advanced');
        await shot('$tag-advanced-filter');
      });

      dbTest('sort editor $tag', (tester) async {
        await pumpDb(tester, mobile: mobile, dark: dark);
        await openView(tester, 'Table', mobile: mobile);
        if (mobile) {
          await tapKey(tester, 'db-settings');
          await tapKey(tester, 'settings-sort');
        } else {
          await tapKey(tester, 'db-sort');
        }
        await tapKey(tester, mobile ? 'pick-Status' : 'pick-Due');
        await shot('$tag-sort');
      });

      dbTest('select editor $tag', (tester) async {
        await pumpDb(tester, mobile: mobile, dark: dark);
        await openView(tester, 'Table', mobile: mobile);
        await tapKey(tester, mobile ? 'cell-Website redesign-Status' : 'cell-Website redesign-Tags');
        await shot('$tag-select-editor');
      });

      dbTest('row page $tag', (tester) async {
        await pumpDb(tester, mobile: mobile, dark: dark);
        await openView(tester, 'Board', mobile: mobile);
        await tapKey(tester, mobile ? 'card-Quarterly report' : 'card-Mobile app launch');
        await shot('$tag-row-page');
      });

      dbTest('view settings $tag', (tester) async {
        await pumpDb(tester, mobile: mobile, dark: dark);
        await openView(tester, 'Table', mobile: mobile);
        if (mobile) {
          await tapKey(tester, 'view-switcher');
          await shot('$tag-view-switcher');
        } else {
          await tapKey(tester, 'db-settings');
          await tapKey(tester, 'settings-layout');
          await shot('$tag-layout');
        }
      });

      dbTest('inline database $tag', (tester) async {
        await pumpDb(tester, mobile: mobile, dark: dark);
        await openPage(tester, 'Planning');
        await shot('$tag-inline');
      });
    }
  }
}
