// Screenshots of the conflict review in the states design/merge/index.html
// shows: restored, switched, text, trash, settings, home and log, at 1440x900
// and 390x844, light and dark, on the real core. The PNGs sit next to the
// mockup's own shots in design/merge/shots/ (app-*.png).
// Regenerate with: flutter test --update-goldens -t golden test/merge_shots_test.dart

@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/sync/conflicts.dart';

import 'harness.dart';
import 'sync_helpers.dart';

const _topline = 'edited this section after you deleted it here';
const _changed = 'changed this page after you deleted it here';

Future<void> shot(String name) async {
  await expectLater(find.byType(Directionality).first, matchesGoldenFile('../../design/merge/shots/app-$name.png'));
}

/// Opens the test vault in the given form factor. pumpApp draws a phone at
/// 375x812; the mockup's phone is 390x844, so the view is resized here.
Future<TestVault> _start(WidgetTester tester, {required bool mobile, required bool dark}) async {
  final tv = await pumpApp(tester, mobile: mobile, dark: dark);
  if (mobile) {
    tester.view.physicalSize = const Size(390 * 2, 844 * 2);
    await settle(tester);
  }
  return tv;
}

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final size = mobile ? 'phone' : 'desktop';
      final theme = dark ? 'dark' : 'light';

      appTest('$size restored $theme', (tester) async {
        await _start(tester, mobile: mobile, dark: dark);
        final other = await openOtherDevice(tester);
        try {
          await pairOther(tester, other);
          final s = await makeSectionPage(tester, other, 'Lisbon trip');
          await makeSubtreeConflict(tester, other, s);
          await openConflictPage(tester, s.page, mobile: mobile);
          expect(find.textContaining(_topline), findsOneWidget);
          await shot('$size-restored-$theme');
        } finally {
          await other.close(tester);
        }
      });

      appTest('$size switched $theme', (tester) async {
        final tv = await _start(tester, mobile: mobile, dark: dark);
        final other = await openOtherDevice(tester);
        try {
          await pairOther(tester, other);
          final s = await makeSectionPage(tester, other, 'Lisbon trip');
          await makeSubtreeConflict(tester, other, s);
          await openConflictPage(tester, s.page, mobile: mobile);
          // The toast is shown after the native removal: wait for that work, or
          // the six-second toast timer runs out during the settle.
          await tapAndSettle(tester, find.text('Use the local version'), waitForBridge: true);
          await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
          expect(find.text('Packing'), findsNothing);
          expect(find.text('Undo'), findsOneWidget);
          await shot('$size-switched-$theme');
        } finally {
          await other.close(tester);
        }
      });

      appTest('$size text $theme', (tester) async {
        final tv = await _start(tester, mobile: mobile, dark: dark);
        final other = await openOtherDevice(tester);
        try {
          await pairOther(tester, other);
          await makeConflict(tester, other, 'Weekly review');
          final page = tv.pages['Weekly review']!;
          await openConflictPage(tester, page, mobile: mobile);
          final c = tv.app.sync.conflictsOf(page).single;
          expect(find.text(c.sides[conflictShownSide(c) == 0 ? 1 : 0].block!.text), findsOneWidget);
          await shot('$size-text-$theme');
        } finally {
          await other.close(tester);
        }
      });

      appTest('$size trash $theme', (tester) async {
        await _start(tester, mobile: mobile, dark: dark);
        final other = await openOtherDevice(tester);
        try {
          await pairOther(tester, other);
          final page = await makePageConflict(tester, other, 'Ideas for the zine', purge: false);
          await openConflictPage(tester, page, mobile: mobile);
          expect(find.textContaining(_changed), findsOneWidget);
          await shot('$size-trash-$theme');
        } finally {
          await other.close(tester);
        }
      });

      appTest('$size settings $theme', (tester) async {
        await _start(tester, mobile: mobile, dark: dark);
        final other = await openOtherDevice(tester);
        try {
          await pairOther(tester, other);
          await makeConflict(tester, other, 'Weekly review');
          final s = await makeSectionPage(tester, other, 'Lisbon trip');
          await makeSubtreeConflict(tester, other, s);
          await openConflictSettings(tester, mobile: mobile);
          expect(key('conflict-auto-resolve'), findsOneWidget);
          expect(key('conflict-page-${s.page}'), findsOneWidget);
          await shot('$size-settings-$theme');
        } finally {
          await other.close(tester);
        }
      });

      appTest('$size home $theme', (tester) async {
        await _start(tester, mobile: mobile, dark: dark);
        final other = await openOtherDevice(tester);
        try {
          await pairOther(tester, other);
          await makeConflict(tester, other, 'Weekly review');
          final s = await makeSectionPage(tester, other, 'Lisbon trip');
          await makeSubtreeConflict(tester, other, s);
          await settle(tester);
          if (mobile) {
            expect(key('conflict-home'), findsOneWidget);
          } else {
            await tapAndSettle(tester, key('conflict-rail-toggle'));
            expect(key('conflict-rail-page-${s.page}'), findsOneWidget);
          }
          await shot('$size-home-$theme');
        } finally {
          await other.close(tester);
        }
      });

      appTest('$size log $theme', (tester) async {
        final tv = await _start(tester, mobile: mobile, dark: dark);
        final other = await openOtherDevice(tester);
        try {
          await pairOther(tester, other);
          final s = await makeSectionPage(tester, other, 'Lisbon trip');
          await makeSubtreeConflict(tester, other, s);
          final c = tv.app.sync.conflictsOf(s.page).single;
          await real(tester, () => tv.app.sync.resolve(c, const Resolution(kind: ResolutionKind.keep, side: 0)));
          await makeConflict(tester, other, 'Weekly review');
          await openConflictSettings(tester, mobile: mobile);
          await tester.ensureVisible(key('conflict-log-${s.page}-${c.id}'));
          await settle(tester);
          expect(key('conflict-log-${s.page}-${c.id}'), findsOneWidget);
          await shot('$size-log-$theme');
        } finally {
          await other.close(tester);
        }
      });
    }
  }
}
