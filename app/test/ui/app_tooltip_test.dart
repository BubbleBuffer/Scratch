// AppTooltip: hover delay, the shortcut from the command registry, hiding on
// leave and Escape, and long-press on touch without pressing the button.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/registry/command_registry.dart';
import 'package:notes_app/ui/ui.dart';

const _command = 'test.tooltip';

Finder get _tip => find.byKey(const ValueKey('app-tooltip'));

Future<void> _pump(WidgetTester tester, {required VoidCallback onTap, bool dark = false}) async {
  tester.view.physicalSize = const Size(600, 400);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.resetPhysicalSize);
  addTearDown(tester.view.resetDevicePixelRatio);
  await tester.pumpWidget(
    AppTheme(
      colors: dark ? AppColors.darkTheme : AppColors.light,
      formFactor: FormFactor.desktop,
      child: WidgetsApp(
        color: const Color(0xFFFFFFFF),
        builder: (context, _) => Overlay(
          initialEntries: [
            OverlayEntry(
              builder: (_) => Align(
                alignment: Alignment.topRight,
                child: IconBtn(
                  Ico.sidebar,
                  key: const ValueKey('btn'),
                  semanticLabel: 'Open sidebar',
                  command: _command,
                  onTap: onTap,
                ),
              ),
            ),
          ],
        ),
      ),
    ),
  );
}

void main() {
  setUpAll(() {
    commandRegistry.register(Command(
      id: _command,
      label: 'Test',
      run: (_) {},
      shortcuts: const [SingleActivator(LogicalKeyboardKey.keyY, control: true, shift: true)],
    ));
  });

  testWidgets('shows after hover with the registry shortcut, hides on leave', (tester) async {
    final semantics = tester.ensureSemantics();
    var taps = 0;
    await _pump(tester, onTap: () => taps++);
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    addTearDown(mouse.removePointer);
    await mouse.addPointer(location: Offset.zero);
    await mouse.moveTo(tester.getCenter(find.byKey(const ValueKey('btn'))));
    await tester.pump(const Duration(milliseconds: 300));
    expect(_tip, findsNothing, reason: 'not before the hover delay');
    await tester.pump(const Duration(milliseconds: 250));
    await tester.pump(const Duration(milliseconds: 200));
    expect(_tip, findsOneWidget);
    expect(find.descendant(of: _tip, matching: find.textContaining('Open sidebar')), findsOneWidget);
    expect(find.descendant(of: _tip, matching: find.textContaining('Ctrl+Shift+Y')), findsOneWidget);

    // Inside the window even though the button sits in the top-right corner.
    final rect = tester.getRect(_tip);
    expect(rect.right, lessThanOrEqualTo(600));
    expect(rect.top, greaterThanOrEqualTo(0));

    await mouse.moveTo(const Offset(10, 390));
    await tester.pump();
    expect(_tip, findsNothing);
    expect(taps, 0);
    // The button keeps its semantic label.
    expect(find.bySemanticsLabel('Open sidebar'), findsOneWidget);
    semantics.dispose();
  });

  testWidgets('Escape hides it', (tester) async {
    await _pump(tester, onTap: () {}, dark: true);
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    addTearDown(mouse.removePointer);
    await mouse.addPointer(location: Offset.zero);
    await mouse.moveTo(tester.getCenter(find.byKey(const ValueKey('btn'))));
    await tester.pump(AppTooltip.hoverDelay);
    await tester.pump(const Duration(milliseconds: 200));
    expect(_tip, findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();
    expect(_tip, findsNothing);
  });

  testWidgets('long-press shows it without pressing the button; a tap still presses', (tester) async {
    var taps = 0;
    await _pump(tester, onTap: () => taps++);
    final btn = find.byKey(const ValueKey('btn'));
    final touch = await tester.startGesture(tester.getCenter(btn));
    await tester.pump(kLongPressTimeout + const Duration(milliseconds: 50));
    await tester.pump(const Duration(milliseconds: 200));
    expect(_tip, findsOneWidget);
    await touch.up();
    await tester.pump();
    expect(taps, 0, reason: 'long-press must not trigger the button');
    expect(_tip, findsOneWidget, reason: 'stays a moment after release');
    await tester.pump(AppTooltip.touchLinger + const Duration(milliseconds: 50));
    expect(_tip, findsNothing);

    await tester.tap(btn);
    await tester.pump();
    expect(taps, 1);
    expect(_tip, findsNothing);
  });

  test('shortcutLabel spells the modifiers', () {
    expect(shortcutLabel(const SingleActivator(LogicalKeyboardKey.keyF, control: true, shift: true)), 'Ctrl+Shift+F');
    expect(commandShortcutLabel(_command), 'Ctrl+Shift+Y');
    expect(commandShortcutLabel('missing.command'), isNull);
  });
}
