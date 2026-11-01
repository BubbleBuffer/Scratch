// The command palette over the command registry: Ctrl+Shift+P opens it, typing
// filters, Enter runs the command through the registry once the palette is gone
// and focus is back, and a disabled command is listed with its reason but never
// runs. The shell's own commands are registered, so their shortcuts show; only
// the test's commands are run. No native core is needed.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/registry/command_registry.dart';
import 'package:notes_app/shell/command_palette.dart';
import 'package:notes_app/shell/commands.dart';
import 'package:notes_app/ui/ui.dart';

final _navKey = GlobalKey<NavigatorState>();
late CommandRegistry _registry;
late FocusNode _home;
late List<String> _ran;

Future<void> _pump(WidgetTester tester, {bool mobile = false}) async {
  tester.view.physicalSize = mobile ? const Size(390, 800) : const Size(1000, 700);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.resetPhysicalSize);
  addTearDown(tester.view.resetDevicePixelRatio);
  _ran = [];
  _registry = CommandRegistry();
  registerShellCommands(_registry);
  _registry.register(
    Command(
      id: 'test.run',
      label: 'Run the test command',
      icon: Ico.code,
      run: (_) => _ran.add('test.run'),
    ),
  );
  _registry.register(
    Command(
      id: 'test.off',
      label: 'Off the test command',
      isEnabled: (_) => false,
      disabledReason: 'Needs a page',
      run: (_) => _ran.add('test.off'),
    ),
  );
  _registry.register(
    Command(
      id: 'test.broken',
      label: 'Disabled without a reason',
      isEnabled: (_) => false,
      run: (_) => _ran.add('test.broken'),
    ),
  );
  _home = FocusNode(debugLabel: 'home');
  addTearDown(_home.dispose);
  await tester.pumpWidget(
    AppTheme(
      colors: AppColors.light,
      formFactor: mobile ? FormFactor.mobile : FormFactor.desktop,
      child: WidgetsApp(
        color: const Color(0xFFFFFFFF),
        navigatorKey: _navKey,
        pageRouteBuilder: <T>(settings, builder) => PageRouteBuilder<T>(
          settings: settings,
          pageBuilder: (context, animation, secondary) => builder(context),
        ),
        // The shell binds every registered shortcut the same way.
        builder: (context, child) => CallbackShortcuts(
          bindings: {
            for (final c in _registry.all)
              for (final s in c.shortcuts)
                s: () {
                  final ctx = _navKey.currentContext;
                  if (ctx != null) _registry.run(c.id, ctx);
                },
          },
          child: child!,
        ),
        home: Focus(
          focusNode: _home,
          autofocus: true,
          child: const SizedBox.expand(),
        ),
      ),
    ),
  );
  await tester.pump();
  expect(_home.hasPrimaryFocus, isTrue);
}

Future<void> _openWithShortcut(WidgetTester tester) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.keyP);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await tester.pumpAndSettle();
}

Finder _row(String id) => find.byKey(ValueKey('palette-command-$id'));

Future<void> _type(WidgetTester tester, String text) async {
  await tester.enterText(find.byType(EditableText), text);
  await tester.pump();
}

void main() {
  testWidgets('Ctrl+Shift+P opens the palette with the registry commands', (
    tester,
  ) async {
    await _pump(tester);
    expect(_registry.of(paletteCommandId)!.shortcuts, hasLength(1));
    await _openWithShortcut(tester);
    expect(find.byType(EditableText), findsOneWidget);
    expect(_row('page.new'), findsOneWidget);
    expect(_row('search'), findsOneWidget);
    expect(_row('test.run'), findsOneWidget);
    // Shortcuts come from the registry; the palette does not list itself.
    expect(find.text('Ctrl+N'), findsOneWidget);
    expect(find.text('Ctrl+K'), findsOneWidget);
    expect(_row(paletteCommandId), findsNothing);
    // Listed by label: New page, Run the test command, Search.
    final tops = [
      for (final id in ['page.new', 'test.run', 'search'])
        tester.getTopLeft(_row(id)).dy,
    ];
    expect(tops[0] < tops[1] && tops[1] < tops[2], isTrue);
    // A second press while it is open does not stack another.
    await _openWithShortcut(tester);
    expect(find.byType(EditableText), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byType(EditableText), findsNothing);
    expect(_home.hasPrimaryFocus, isTrue);
    expect(_ran, isEmpty);
    // And it opens again afterwards.
    await _openWithShortcut(tester);
    expect(find.byType(EditableText), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
  });

  testWidgets('opening, filtering and running a command works', (tester) async {
    await _pump(tester);
    await _openWithShortcut(tester);
    await _type(tester, 'rtc');
    expect(_row('test.run'), findsOneWidget);
    expect(_row('page.new'), findsNothing);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(_ran, ['test.run']);
    expect(find.byType(EditableText), findsNothing);
    expect(_home.hasPrimaryFocus, isTrue);
  });

  testWidgets('a tap runs a command once', (tester) async {
    await _pump(tester);
    await _openWithShortcut(tester);
    await tester.tap(_row('test.run'));
    await tester.pumpAndSettle();
    expect(_ran, ['test.run']);
  });

  testWidgets('a disabled command shows its reason and does not run', (
    tester,
  ) async {
    await _pump(tester);
    await _openWithShortcut(tester);
    await _type(tester, 'off the');
    expect(_row('test.off'), findsOneWidget);
    expect(find.text('Needs a page'), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.tap(_row('test.off'));
    await tester.pumpAndSettle();
    expect(_ran, isEmpty);
    // Still open; a disabled command without a reason says the fallback.
    expect(find.byType(EditableText), findsOneWidget);
    await _type(tester, 'without');
    expect(find.text(paletteDisabledFallback), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pumpAndSettle();
    expect(_ran, isEmpty);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byType(EditableText), findsNothing);
  });

  testWidgets('the palette opens full screen on a phone', (tester) async {
    await _pump(tester, mobile: true);
    openCommandPalette(_navKey.currentContext!, registry: _registry);
    await tester.pumpAndSettle();
    expect(tester.getSize(find.byType(EditableText)).width, lessThan(390));
    expect(find.text('Cancel'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(find.byType(EditableText), findsNothing);
    expect(_home.hasPrimaryFocus, isTrue);
  });
}
