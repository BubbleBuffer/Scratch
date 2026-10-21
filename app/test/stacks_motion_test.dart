import 'dart:ui' show CheckedState;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/ui/ui.dart';

Widget fixture(Widget child, {bool reduced = false}) => MediaQuery(
  data: MediaQueryData(size: const Size(800, 600), disableAnimations: reduced),
  child: AppTheme(
    colors: AppColors.light,
    formFactor: FormFactor.desktop,
    child: Directionality(
      textDirection: TextDirection.ltr,
      child: FocusScope(child: Center(child: child)),
    ),
  ),
);

void main() {
  test(
    'display type preserves condensed and weight axes with optical sizing',
    () {
      Map<String, double> axes(double size, {int? weight}) => {
        for (final axis in tsDisplay(
          TypeSpec(size, size, 640),
          AppColors.light.textPrimary,
          weight: weight,
        ).fontVariations!)
          axis.axis: axis.value,
      };
      expect(axes(44), {'opsz': 44, 'wdth': 90, 'wght': 640});
      expect(axes(10), {'opsz': 12, 'wdth': 90, 'wght': 640});
      expect(axes(110, weight: 700), {'opsz': 96, 'wdth': 90, 'wght': 700});
    },
  );
  testWidgets('motion follows the device reduced-animation preference', (
    tester,
  ) async {
    late MotionDurations durations;
    Widget sample(bool reduced) => fixture(
      Builder(
        builder: (context) {
          durations = Motion.of(context);
          return const SizedBox();
        },
      ),
      reduced: reduced,
    );
    await tester.pumpWidget(sample(false));
    expect(
      [durations.fast, durations.standard, durations.sheet],
      [
        const Duration(milliseconds: 150),
        const Duration(milliseconds: 220),
        const Duration(milliseconds: 250),
      ],
    );
    await tester.pumpWidget(sample(true));
    expect(durations.reduced, isTrue);
    expect(
      [durations.fast, durations.standard, durations.sheet],
      [Duration.zero, Duration.zero, Duration.zero],
    );
  });

  testWidgets('page route durations honour reduced motion and desktop swaps', (
    tester,
  ) async {
    late PageRouteBuilder<void> route;
    Widget sample({required bool reduced, required bool mobile}) => fixture(
      Builder(
        builder: (context) {
          route = pageRoute<void>(
            (_) => const SizedBox(),
            mobile: mobile,
            motion: Motion.of(context),
          ) as PageRouteBuilder<void>;
          return const SizedBox();
        },
      ),
      reduced: reduced,
    );
    await tester.pumpWidget(sample(reduced: false, mobile: true));
    expect(route.transitionDuration, const Duration(milliseconds: 250));
    expect(route.reverseTransitionDuration, const Duration(milliseconds: 220));
    route.dispose();
    await tester.pumpWidget(sample(reduced: true, mobile: true));
    expect(route.transitionDuration, Duration.zero);
    expect(route.reverseTransitionDuration, Duration.zero);
    route.dispose();
    await tester.pumpWidget(sample(reduced: false, mobile: false));
    expect(route.transitionDuration, Duration.zero);
    expect(route.reverseTransitionDuration, Duration.zero);
    route.dispose();
  });

  testWidgets(
    'a shared control lifts on hover and sinks during a pointer press',
    (tester) async {
      const content = ValueKey('control-content');
      var taps = 0;
      await tester.pumpWidget(
        fixture(
          Hover(
            onTap: () => taps++,
            child: const SizedBox(key: content, width: 80, height: 40),
          ),
        ),
      );
      final baseline = tester.getTopLeft(find.byKey(content));
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      await mouse.moveTo(tester.getCenter(find.byKey(content)));
      await tester.pumpAndSettle();
      expect(
        tester.getTopLeft(find.byKey(content)),
        baseline + const Offset(0, -1),
      );
      await mouse.down(tester.getCenter(find.byKey(content)));
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pumpAndSettle();
      expect(
        tester.getTopLeft(find.byKey(content)),
        baseline + const Offset(0, 1),
      );
      await mouse.up();
      await tester.pumpAndSettle();
      expect(taps, 1);
      await mouse.moveTo(Offset.zero);
      await tester.pumpAndSettle();
      expect(tester.getTopLeft(find.byKey(content)), baseline);
      await mouse.removePointer();
    },
  );

  testWidgets(
    'reduced motion keeps the target still and keyboard activation works',
    (tester) async {
      const content = ValueKey('reduced-content');
      var taps = 0;
      await tester.pumpWidget(
        fixture(
          Hover(
            onTap: () => taps++,
            child: const SizedBox(key: content, width: 80, height: 40),
          ),
          reduced: true,
        ),
      );
      final baseline = tester.getTopLeft(find.byKey(content));
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      await mouse.moveTo(tester.getCenter(find.byKey(content)));
      await tester.pump();
      expect(tester.getTopLeft(find.byKey(content)), baseline);
      await mouse.down(tester.getCenter(find.byKey(content)));
      await tester.pump(const Duration(milliseconds: 300));
      expect(tester.getTopLeft(find.byKey(content)), baseline);
      await mouse.up();
      await tester.pump();
      expect(taps, 1);
      Focus.of(tester.element(find.byKey(content))).requestFocus();
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      expect(taps, 3);
      await mouse.removePointer();
    },
  );

  testWidgets(
    'the animated task box retains its hit area and checked semantics',
    (tester) async {
      final semantics = tester.ensureSemantics();
      try {
        var checked = false;
        await tester.pumpWidget(
          fixture(
            StatefulBuilder(
              builder: (context, setState) => StacksCheckbox(
                checked: checked,
                onChanged: (value) => setState(() => checked = value),
              ),
            ),
          ),
        );
        final box = find.byType(StacksCheckbox);
        expect(tester.getSize(box), const Size(22, 22));
        expect(
          tester.getSemantics(box).flagsCollection.isChecked,
          CheckedState.isFalse,
        );
        await tester.tap(box);
        await tester.pumpAndSettle();
        expect(checked, isTrue);
        expect(tester.getSize(box), const Size(22, 22));
        expect(
          tester.getSemantics(box).flagsCollection.isChecked,
          CheckedState.isTrue,
        );
        Focus.of(tester.element(find.byType(AnimatedContainer).last))
            .requestFocus();
        await tester.pump();
        await tester.sendKeyEvent(LogicalKeyboardKey.space);
        await tester.pumpAndSettle();
        expect(checked, isFalse);
        expect(
          tester.getSemantics(box).flagsCollection.isChecked,
          CheckedState.isFalse,
        );
      } finally {
        semantics.dispose();
      }
    },
  );
}
