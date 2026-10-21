// Golden sources for a first run: selected-day Today on a fresh vault, mobile Pages
// tab and the Getting started page. 1440x900 and 375x812, light and dark.
// Regenerate with: flutter test --update-goldens test/home_first_run_golden_test.dart

@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/getting_started.dart';

import 'harness.dart';

Future<void> shot(String name) async {
  await expectLater(
    find.byType(Directionality).first,
    matchesGoldenFile('../screenshots/$name.png'),
  );
}

Finder key(String k) => find.byKey(ValueKey(k));

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';

      appTest('first-run Today $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark, fresh: true);
        await settle(tester);
        await shot('first-run-$tag-home');
      });

      if (mobile) {
        appTest('mobile pages $tag', (tester) async {
          final f = await pumpApp(
            tester,
            mobile: mobile,
            dark: dark,
            fresh: true,
          );
          await tapAndSettle(tester, key('nav-pages'));
          final tasks = f.app.tree
              .childrenOf(null)!
              .firstWhere((p) => p.title == 'Tasks')
              .id;
          await tapAndSettle(tester, key('pages-toggle-$tasks'));
          await shot('first-run-$tag-pages');
        });
      }

      appTest('getting started $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark, fresh: true);
        if (mobile) {
          await tapAndSettle(tester, key('nav-pages'));
          await tapAndSettle(
            tester,
            find.descendant(
              of: key('pages-list'),
              matching: find.text(gettingStartedTitle),
            ),
          );
        } else {
          await tapAndSettle(
            tester,
            find.descendant(
              of: key('sidebar'),
              matching: find.text(gettingStartedTitle),
            ),
          );
        }
        await settle(tester);
        await shot('first-run-$tag-getting-started');
      });
    }
  }
}
