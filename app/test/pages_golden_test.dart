// Golden screenshots of the Phase 2 page features: a page with a cover and
// icon and every new block, the emoji picker, the page menu, backlinks,
// multi-block selection and the mobile toolbar's rows, at 1440x900 and
// 375x812, light and dark.
// Regenerate with: flutter test --update-goldens test/pages_golden_test.dart

@Tags(['golden'])
library;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'harness.dart';
import 'pages_fixture.dart';

Future<void> shot(String name) async {
  await expectLater(find.byType(Directionality).first, matchesGoldenFile('../screenshots/$name.png'));
}

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';

      appTest('rich page $tag', (tester) async {
        final tv = await pumpApp(tester, mobile: mobile, dark: dark);
        await createRichPage(tester, tv);
        await openByTitle(tester, richTitle);
        await shot('$tag-rich-page');
        // Further down: toggles open, columns, media and links. On a phone
        // the toggle sits just below the visible area: scroll it in first.
        final toggle = find.byKey(const ValueKey('toggle-r06'), skipOffstage: false);
        await Scrollable.ensureVisible(
          tester.element(toggle),
          alignmentPolicy: ScrollPositionAlignmentPolicy.keepVisibleAtEnd,
        );
        await settle(tester);
        await tester.tap(toggle);
        await settle(tester);
        await tester.drag(find.byType(CustomScrollView).last, Offset(0, mobile ? -560 : -420));
        // Finish the scrollbar's delayed fade on Flutter's fake clock.
        await tester.pump(const Duration(seconds: 2));
        await settle(tester);
        await shot('$tag-rich-page-blocks');
      });

      appTest('emoji picker $tag', (tester) async {
        final tv = await pumpApp(tester, mobile: mobile, dark: dark);
        await createRichPage(tester, tv);
        await openByTitle(tester, richTitle);
        await tester.tap(find.byKey(const ValueKey('page-icon')));
        await settle(tester);
        await shot('$tag-emoji-picker');
      });

      appTest('page menu $tag', (tester) async {
        final tv = await pumpApp(tester, mobile: mobile, dark: dark);
        await createRichPage(tester, tv);
        await openByTitle(tester, richTitle);
        await tester.tap(find.byKey(const ValueKey('page-menu')));
        await settle(tester);
        await shot('$tag-page-menu');
      });

      appTest('backlinks $tag', (tester) async {
        final tv = await pumpApp(tester, mobile: mobile, dark: dark);
        await createRichPage(tester, tv);
        await openByTitle(tester, 'Recipes');
        await shot('$tag-backlinks');
      });

      appTest('multi-select $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        await openShowcase(tester);
        if (mobile) {
          await tester.longPress(editableStarting('Bulleted list item'));
          await settle(tester);
          await tester.tap(find.byKey(const ValueKey('sheet-select')));
          await settle(tester);
          await tester.tap(editableStarting('Another top-level'));
          await settle(tester);
          await tester.tap(editableStarting('Nested item'));
          await settle(tester);
        } else {
          final a = editableStarting('Bulleted list item');
          await focusAt(tester, a, 0);
          await tester.sendKeyDownEvent(LogicalKeyboardKey.shift);
          await tester.tap(editableStarting('Another top-level'));
          await tester.sendKeyUpEvent(LogicalKeyboardKey.shift);
          await settle(tester);
        }
        await shot('$tag-multi-select');
      });

      if (mobile) {
        appTest('keyboard toolbar rows $tag', (tester) async {
          await pumpApp(tester, mobile: mobile, dark: dark);
          await openShowcase(tester);
          await focusAt(tester, editableStarting('Plain paragraph'), 5);
          await showKeyboard(tester);
          await tester.tap(find.byKey(const ValueKey('kb-more')));
          await settle(tester);
          await shot('$tag-keyboard-more');
          await tester.tap(find.byKey(const ValueKey('kb-back')));
          await settle(tester);
          await tester.tap(find.byKey(const ValueKey('kb-format')));
          await settle(tester);
          await shot('$tag-keyboard-format');
        });
      } else {
        appTest('mention menu $tag', (tester) async {
          await pumpApp(tester, mobile: mobile, dark: dark);
          await openShowcase(tester);
          final b = editableStarting('Plain paragraph');
          await focusAt(tester, b, editState(tester, b).widget.controller.text.length);
          await typeText(tester, ' @');
          await shot('$tag-mention-menu');
        });

        appTest('block colour menu $tag', (tester) async {
          await pumpApp(tester, mobile: mobile, dark: dark);
          await openShowcase(tester);
          final b = editableStarting('Bulleted list item');
          final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
          await g.addPointer(location: Offset.zero);
          await g.moveTo(tester.getCenter(b));
          await settle(tester);
          await tester.tap(find.byKey(const ValueKey('grip-sc05')));
          await settle(tester);
          await tester.tap(find.byKey(const ValueKey('menu-color')));
          await settle(tester);
          await g.moveTo(tester.getCenter(find.byKey(const ValueKey('color-blue_background'))));
          await settle(tester);
          await shot('$tag-block-color');
          await g.removePointer();
        });
      }
    }
  }
}
