// Golden screenshots of home, a page and the editor menus, at 1440x900
// (desktop) and 375x812 (mobile), light and dark, with real fonts.
// Regenerate with: flutter test --update-goldens test/golden_test.dart

@Tags(['golden'])
library;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:notes_app/addons/search_card/query_panel.dart';
import 'package:notes_app/addons/search_card/query_watch.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/state/app_state.dart';
import 'package:notes_app/ui/buttons.dart';
import 'package:notes_app/ui/sheet.dart';

import 'addons/search_card/search_card_fixture.dart';

Future<void> shot(String name) async {
  await expectLater(find.byType(Directionality).first, matchesGoldenFile('../screenshots/$name.png'));
}

const _openWork = '#work type:task -status:done';

/// A page with the chip under its intro line, counting the open #work tasks
/// of the sample data, with its label in.
Future<void> _openChipPage(
  WidgetTester tester, {
  required bool mobile,
  required bool dark,
}) async {
  await pumpApp(tester, mobile: mobile, dark: dark);
  await seedSearchCardPage(tester);
  final page = await seedPage(tester, 'Week 43');
  await appendBlocks(tester, page, [
    (Kinds.paragraph, 'The open work for this week, from the tasks tagged #work:'),
  ]);
  await seedChip(tester, page, 'Open work', query: _openWork, start: 0, end: 9);
  await openChipPage(tester, page, 'Week 43', text: 'Open work', query: _openWork);
}

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';

      appTest('home $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        await shot('$tag-home');
      });

      appTest('page $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        await openShowcase(tester);
        await shot('$tag-page');
      });

      appTest('slash menu $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        await openShowcase(tester);
        final b = editableStarting('Plain paragraph');
        await focusAt(tester, b, editState(tester, b).widget.controller.text.length);
        if (mobile) await showKeyboard(tester);
        await tester.sendKeyEvent(LogicalKeyboardKey.enter);
        await settle(tester);
        await typeText(tester, '/');
        await shot('$tag-slash-menu');
      });

      appTest('${mobile ? 'keyboard' : 'selection'} toolbar $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        await openShowcase(tester);
        final b = editableStarting('This page has');
        final t = editState(tester, b).widget.controller.text;
        final i = t.indexOf('every kind');
        await focusAt(tester, b, i, extent: i + 'every kind'.length);
        if (mobile) await showKeyboard(tester);
        await settle(tester);
        await shot('$tag-${mobile ? 'keyboard' : 'selection'}-toolbar');
      });

      appTest('${mobile ? 'bottom sheet' : 'block menu'} $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        await openShowcase(tester);
        final b = editableStarting('Bulleted list item');
        if (mobile) {
          await tester.longPress(b);
          await settle(tester);
          await shot('$tag-bottom-sheet');
        } else {
          final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
          await g.addPointer(location: Offset.zero);
          await g.moveTo(tester.getCenter(b));
          await settle(tester);
          final grip = find.byWidgetPredicate((w) => w.key is ValueKey && '${(w.key as ValueKey).value}'.startsWith('grip-sc05'));
          await g.moveTo(tester.getCenter(grip));
          await settle(tester);
          await tester.tap(grip);
          await settle(tester);
          await g.moveTo(tester.getCenter(find.byKey(const ValueKey('menu-duplicate'))));
          await settle(tester);
          await shot('$tag-block-menu');
          await g.removePointer();
        }
      });

      appTest('search card $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        await seedSearchCardPage(tester);
        await shot('search-card-$tag');
      });
    }
  }

  appTest('search card after tick', (tester) async {
    await pumpApp(tester, mobile: false);
    final page = await seedSearchCardPage(tester);
    await tester.tap(
      find
          .descendant(of: inCard(page.tasks), matching: find.byType(StacksCheckbox))
          .first,
    );
    // The row leaves once the core's live result arrives; the message follows it.
    await settleUntil(
      tester,
      () => find.text('Marked done, so it left this list.').evaluate().isNotEmpty,
    );
    await settle(tester);
    await shot('search-card-after-tick-desktop-light');
  });

  appTest('search card empty', (tester) async {
    await pumpApp(tester, mobile: false);
    await seedSearchCardPage(tester, boardEmpty: true);
    await shot('search-card-empty-desktop-light');
  });

  // The chip (Form B): the paragraph, its results popover on desktop and its
  // sheet on phone, in both themes.
  appTest('chip paragraph desktop light', (tester) async {
    await _openChipPage(tester, mobile: false, dark: false);
    await shot('chip-desktop-light-paragraph');
  });

  for (final dark in [false, true]) {
    final theme = dark ? 'dark' : 'light';
    appTest('chip popover desktop $theme', (tester) async {
      await _openChipPage(tester, mobile: false, dark: dark);
      await tapChip(tester, 'Open work');
      await settle(tester);
      await shot('chip-desktop-$theme-popover');
    });
    appTest('chip sheet mobile $theme', (tester) async {
      await _openChipPage(tester, mobile: true, dark: dark);
      await tapChip(tester, 'Open work');
      await settle(tester);
      await shot('chip-mobile-$theme-sheet');
    });
  }

  // The sheet after a tick: the Undo message sits above the footer. The sheet
  // is opened on its own watch; the row leaves on the core's live result.
  appTest('chip sheet after tick, undo above footer', (tester) async {
    await pumpApp(tester, mobile: true);
    await seedSearchCardPage(tester);
    final context = tester.element(find.byType(Overlay).first);
    final watch = QueryWatch(
      vault: AppScope.of(context).vault,
      tasksOn: true,
      query: _openWork,
    )..start();
    showQueryPanel(
      context,
      watch: watch,
      anchor: const Rect.fromLTWH(600, 200, 40, 30),
      onOpenInSearch: () {},
      onShowAsBlock: () {},
    );
    await settleUntil(tester, () => watch.ready);
    await settle(tester);

    // The page behind the sheet draws the same row, so tap the sheet's copy.
    final hit = watch.hits.first;
    final sheet = find.byType(SheetSurface);
    await tapAndSettle(
      tester,
      find.descendant(
        of: sheet,
        matching: find.byKey(ValueKey('query-check-${hit.id}')),
      ),
    );
    await settleUntil(
      tester,
      () => find.byKey(const ValueKey('query-panel-message')).evaluate().isNotEmpty,
    );
    await settle(tester);
    await shot('chip-mobile-light-sheet-undo');
    watch.dispose();
  });

  // A block kind this version does not know shows the unsupported card, in
  // both themes, beside the paragraph around it.
  for (final dark in [false, true]) {
    final theme = dark ? 'dark' : 'light';
    appTest('unsupported block desktop $theme', (tester) async {
      await pumpApp(tester, mobile: false, dark: dark);
      final page = await seedPage(tester, 'Week 43');
      await appendBlocks(tester, page, [
        (Kinds.paragraph, 'A block from a newer version sits below this line:'),
        ('future_kind', 'tag:work'),
      ]);
      await openPageNamed(tester, page, 'Week 43');
      await settleUntil(
        tester,
        () => find
            .text('Not supported on this version of Scratch')
            .evaluate()
            .isNotEmpty,
      );
      await settle(tester);
      await shot('unsupported-desktop-$theme');
    });
  }
}
