import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/ui/mobile_navigation_gesture.dart';
import 'package:notes_app/ui/selection.dart';
import 'package:notes_app/ui/tokens.dart';

void main() {
  Widget surface(Widget child) => MediaQuery(
    data: const MediaQueryData(size: Size(390, 844)),
    child: Directionality(textDirection: TextDirection.ltr, child: child),
  );

  testWidgets('inner horizontal scrollable wins the gesture arena', (t) async {
    final scroll = ScrollController();
    addTearDown(scroll.dispose);
    var starts = 0, cancels = 0;
    await t.pumpWidget(
      surface(
        MobileNavigationGesture(
          onStart: () => starts++,
          onCancel: () => cancels++,
          onUpdate: (_) {},
          onEnd: (_) {},
          child: SingleChildScrollView(
            controller: scroll,
            scrollDirection: Axis.horizontal,
            child: const SizedBox(
              width: 1800,
              child: ColoredBox(color: Color(0xFFFFFFFF)),
            ),
          ),
        ),
      ),
    );
    await t.drag(find.byType(SingleChildScrollView), const Offset(-200, 0));
    await t.pumpAndSettle();
    expect(scroll.offset, greaterThan(0));
    expect(starts, 0);
    expect(cancels, 0);
  });

  testWidgets('inner horizontal drag handler wins the gesture arena', (
    t,
  ) async {
    var navigation = 0, resize = 0;
    await t.pumpWidget(
      surface(
        MobileNavigationGesture(
          onStart: () => navigation++,
          onUpdate: (_) {},
          onEnd: (_) {},
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onHorizontalDragUpdate: (_) => resize++,
            child: const SizedBox.expand(),
          ),
        ),
      ),
    );
    await t.drag(find.byType(GestureDetector), const Offset(200, 0));
    expect(resize, greaterThan(0));
    expect(navigation, 0);
  });

  testWidgets(
    'vertical scrolling and mainly vertical drags keep navigation idle',
    (t) async {
      final scroll = ScrollController();
      addTearDown(scroll.dispose);
      var starts = 0;
      await t.pumpWidget(
        surface(
          MobileNavigationGesture(
            onStart: () => starts++,
            onUpdate: (_) {},
            onEnd: (_) {},
            child: SingleChildScrollView(
              controller: scroll,
              child: const SizedBox(
                height: 2400,
                child: ColoredBox(color: Color(0xFFFFFFFF)),
              ),
            ),
          ),
        ),
      );
      await t.drag(find.byType(SingleChildScrollView), const Offset(80, -300));
      await t.pumpAndSettle();
      expect(scroll.offset, greaterThan(0));
      expect(starts, 0);
      // Even without a vertical recognizer, a diagonal mostly vertical drag
      // must not turn into navigation once its horizontal distance exceeds slop.
      await t.pumpWidget(
        surface(
          MobileNavigationGesture(
            onStart: () => starts++,
            onUpdate: (_) {},
            onEnd: (_) {},
            child: const SizedBox.expand(),
          ),
        ),
      );
      await t.drag(
        find.byType(MobileNavigationGesture),
        const Offset(80, -300),
      );
      expect(starts, 0);
    },
  );

  testWidgets('dynamic interaction blocker is read at pointer down', (t) async {
    var blocked = true, starts = 0;
    await t.pumpWidget(
      surface(
        MobileNavigationGesture(
          canStart: () => !blocked,
          onStart: () => starts++,
          onUpdate: (_) {},
          onEnd: (_) {},
          child: const SizedBox.expand(),
        ),
      ),
    );
    await t.drag(find.byType(MobileNavigationGesture), const Offset(200, 0));
    expect(starts, 0);
    blocked = false;
    await t.drag(find.byType(MobileNavigationGesture), const Offset(200, 0));
    expect(starts, 1);
  });

  testWidgets('focused text selection blocks swipes outside the field', (
    t,
  ) async {
    final controller = TextEditingController(text: 'Selected text');
    final focus = FocusNode();
    addTearDown(controller.dispose);
    addTearDown(focus.dispose);
    var starts = 0;
    await t.pumpWidget(
      WidgetsApp(
        color: const Color(0xFFFFFFFF),
        builder: (_, child) => AppTheme(
          colors: AppColors.light,
          formFactor: FormFactor.mobile,
          child: child!,
        ),
        onGenerateRoute: (_) => PageRouteBuilder<void>(
          pageBuilder: (_, _, _) => MobileNavigationGesture(
            onStart: () => starts++,
            onUpdate: (_) {},
            onEnd: (_) {},
            child: Column(
              children: [
                EditableText(
                  controller: controller,
                  focusNode: focus,
                  style: const TextStyle(fontSize: 16),
                  cursorColor: const Color(0xFF000000),
                  backgroundCursorColor: const Color(0xFF000000),
                  showSelectionHandles: true,
                  selectionControls: noteSelectionControls,
                ),
                const Expanded(child: SizedBox.expand()),
              ],
            ),
          ),
        ),
      ),
    );
    focus.requestFocus();
    controller.selection = const TextSelection(baseOffset: 0, extentOffset: 8);
    await t.pump();
    await t.dragFrom(const Offset(100, 200), const Offset(200, 0));
    expect(starts, 0);
    controller.selection = const TextSelection.collapsed(offset: 8);
    await t.pump();
    await t.dragFrom(const Offset(100, 200), const Offset(200, 0));
    expect(starts, 1);
    // A collapsed caret can still display a handle after touch selection.
    final editable = t.state<EditableTextState>(find.byType(EditableText));
    focus.requestFocus();
    await t.pump();
    editable.userUpdateTextEditingValue(
      controller.value.copyWith(
        selection: const TextSelection.collapsed(offset: 7),
      ),
      SelectionChangedCause.longPress,
    );
    editable.selectionOverlay!.showHandles();
    await t.pump();
    expect(editable.selectionOverlay!.handlesAreVisible, isTrue);
    await t.dragFrom(const Offset(100, 200), const Offset(200, 0));
    expect(starts, 1);
    focus.unfocus();
    await t.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.android));

  testWidgets('a route covered by a popup cannot start navigation', (t) async {
    final navigator = GlobalKey<NavigatorState>();
    var starts = 0;
    await t.pumpWidget(
      WidgetsApp(
        color: const Color(0xFFFFFFFF),
        navigatorKey: navigator,
        onGenerateRoute: (_) => PageRouteBuilder<void>(
          pageBuilder: (_, _, _) => MobileNavigationGesture(
            onStart: () => starts++,
            onUpdate: (_) {},
            onEnd: (_) {},
            child: const SizedBox.expand(),
          ),
        ),
      ),
    );
    navigator.currentState!.push(_NonBlockingPopup());
    await t.pumpAndSettle();
    // The popup deliberately leaves hit testing to the underlying route:
    // the navigation gate still observes that its route is not current.
    await t.dragFrom(const Offset(100, 200), const Offset(200, 0));
    expect(starts, 0);
    navigator.currentState!.pop();
    await t.pumpAndSettle();
    await t.dragFrom(const Offset(100, 200), const Offset(200, 0));
    expect(starts, 1);
  });
}

class _NonBlockingPopup extends PopupRoute<void> {
  @override
  Color? get barrierColor => null;
  @override
  bool get barrierDismissible => false;
  @override
  String? get barrierLabel => null;
  @override
  Duration get transitionDuration => Duration.zero;
  @override
  Widget buildPage(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
  ) => const IgnorePointer(child: SizedBox.expand());
}
