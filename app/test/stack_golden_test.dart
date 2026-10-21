// Golden screenshots of the desktop pile: one page, three pages with a
// paper edge, the fan-out list, light and dark; and the UX review's flow
// step by step (light). Files: screenshots/stack-<theme>-<scene>.png and
// screenshots/stack-flow-<step>.png.
// Regenerate with: flutter test --update-goldens test/stack_golden_test.dart

@Tags(['golden'])
library;

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;

import 'db_harness.dart';
import 'stack_flow.dart';

Future<String> _id(WidgetTester tester, String title) async {
  final pages = await core(tester, (v) => v.listChildren(null));
  return pages.firstWhere((p) => p.title == title).id;
}

Future<void> _open(
  WidgetTester tester,
  String title, {
  bool stack = true,
}) async {
  final id = await _id(tester, title);
  final s = stackOf(tester);
  final ctx = s.pages.isEmpty
      ? tester.element(find.byType(HomeScreen))
      : editorOf(tester, s.topId!).context;
  unawaited(nav.openPage(ctx, id, title, stack: stack));
  await settle(tester);
}

void main() {
  setUpAll(initCore);

  for (final dark in [false, true]) {
    final tag = 'stack-${dark ? 'dark' : 'light'}';

    dbTest('one page $tag', (tester) async {
      await pumpDb(tester, mobile: false, dark: dark);
      await _open(tester, 'Weekly review', stack: false);
      await shot('$tag-depth1');
    });

    dbTest('three pages $tag', (tester) async {
      await pumpDb(tester, mobile: false, dark: dark);
      await _open(tester, 'Weekly review', stack: false);
      await _open(tester, 'Garden plan');
      await _open(tester, 'Reading notes');
      expect(byKey('stack-edge-0'), findsOneWidget);
      await shot('$tag-depth3');
    });

    dbTest('fan-out $tag', (tester) async {
      await pumpDb(tester, mobile: false, dark: dark);
      await _open(tester, 'Weekly review', stack: false);
      await _open(tester, 'Garden plan');
      await _open(tester, 'Reading notes');
      await tapKey(tester, 'stack-edge-0');
      expect(byKey('stack-fan'), findsOneWidget);
      await shot('$tag-fan-out');
    });
  }

  dbTest('the UX review flow', (tester) async {
    await pumpDb(tester, mobile: false);
    await runStackFlow(tester, (step) async {
      // Elapsed real time must not age the header in a slow golden run. Resetting
      // Clock alone does not rebuild an already-rendered header, so mark the
      // top editor for rebuild and pump before the shot.
      Clock.fixedAt = fixedNow;
      (editorOf(tester, stackOf(tester).topId!).context as Element)
          .markNeedsBuild();
      await tester.pump();
      await shot('stack-flow-$step');
    });
  });
}
