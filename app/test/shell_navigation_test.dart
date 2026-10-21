import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/api.dart';
import 'package:notes_app/registry/addon_registry.dart';
import 'package:notes_app/shell/addons.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';

Finder key(String name) => find.byKey(ValueKey(name));

Future<void> pumpFrame(WidgetTester tester, String destination) async {
  tester.view.physicalSize = const Size(390, 844);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    AppTheme(
      colors: AppColors.light,
      formFactor: FormFactor.mobile,
      child: WidgetsApp(
        color: AppColors.light.bgPrimary,
        pageRouteBuilder: <T>(settings, builder) => PageRouteBuilder<T>(
          settings: settings,
          pageBuilder: (context, _, _) => builder(context),
        ),
        home: ShellMobileFrame(
          destinationId: destination,
          onPages: () {},
          onToday: () {},
          child: const SizedBox.expand(),
        ),
      ),
    ),
  );
  await tester.pump();
}

void main() {
  setUpAll(initCore);

  test('Tasks boot registers Calendar navigation and shared task action', () {
    final ctx = AddonContext();
    bootAddons(const [TasksAddon()], ctx);
    final entry = ctx.shellNavigation.of('calendar')!;
    expect(entry.label, 'Calendar');
    expect(entry.routeName, calendarRouteName);
    expect(entry.sidebar, isTrue);
    expect(entry.mobileDock, isTrue);
    expect(ctx.shellPrimaryActions.of('task.new')!.destinations, {
      'today',
      'calendar',
    });
    final command = ctx.commands.of('calendar.open')!;
    expect(command.shortcuts, [
      const SingleActivator(
        LogicalKeyboardKey.keyC,
        control: true,
        shift: true,
      ),
    ]);
    ctx.noteTypes.dispose();
  });

  testWidgets(
    'dock includes only opted-in entries in stable registered order',
    (tester) async {
      final previous = activeAddonContext;
      final ctx = AddonContext();
      activeAddonContext = ctx;
      addTearDown(() {
        activeAddonContext = previous;
        ctx.noteTypes.dispose();
      });
      var opened = '';
      for (final values in [
        ('last', 20, true),
        ('hidden', -10, false),
        ('first', 10, true),
      ]) {
        ctx.shellNavigation.register(
          AddonShellNavigation(
            id: values.$1,
            label: values.$1,
            icon: Ico.page,
            routeName: '/${values.$1}',
            order: values.$2,
            mobileDock: values.$3,
            open: (_) {
              opened = values.$1;
            },
          ),
        );
      }
      await pumpFrame(tester, 'today');
      expect(key('nav-hidden'), findsNothing);
      expect(
        tester.getRect(key('nav-first')).left,
        lessThan(tester.getRect(key('nav-last')).left),
      );
      await tester.tap(key('nav-first'));
      expect(opened, 'first');
      expect(key('mobile-create'), findsNothing);
    },
  );

  testWidgets(
    'floating action dispatches through registration on Today and Calendar',
    (tester) async {
      final previous = activeAddonContext;
      final ctx = AddonContext();
      activeAddonContext = ctx;
      addTearDown(() {
        activeAddonContext = previous;
        ctx.noteTypes.dispose();
      });
      var created = 0;
      ctx.shellPrimaryActions.register(
        AddonShellPrimaryAction(
          id: 'example.new',
          label: 'Create example',
          destinations: const {'today', 'example'},
          run: (_) {
            created++;
          },
        ),
      );
      await pumpFrame(tester, 'today');
      await tester.tap(key('mobile-create'));
      expect(created, 1);
      await pumpFrame(tester, 'example');
      await tester.tap(key('mobile-create'));
      expect(created, 2);
      await pumpFrame(tester, 'pages');
      expect(key('mobile-create'), findsNothing);
      expect(key('new-page'), findsOneWidget);
    },
  );

  testWidgets(
    'boot without Tasks removes Calendar and task create action from phone',
    (tester) async {
      final previous = activeAddonContext;
      final ctx = AddonContext();
      bootAddons(enabledAddons({'tasks'}), ctx);
      activeAddonContext = ctx;
      addTearDown(() {
        activeAddonContext = previous;
        ctx.noteTypes.dispose();
      });
      await pumpFrame(tester, 'today');
      expect(key('nav-calendar'), findsNothing);
      expect(key('mobile-create'), findsNothing);
      for (final name in ['nav-pages', 'nav-today', 'nav-search']) {
        expect(key(name), findsOneWidget);
      }
    },
  );

  appTest('Calendar sidebar entry opens and selects its own destination', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    expect(key('sidebar-calendar'), findsOneWidget);
    expect(
      tester.getRect(key('sidebar-home')).top,
      lessThan(tester.getRect(key('sidebar-calendar')).top),
    );
    expect(
      tester.getRect(key('sidebar-calendar')).top,
      lessThan(tester.getRect(key('sidebar-agent')).top),
    );
    await tapAndSettle(tester, key('sidebar-calendar'));
    expect(find.byType(VaultCalendarScreen), findsOneWidget);
    final selected = tester.widget<Hover>(
      find.descendant(
        of: key('sidebar-calendar'),
        matching: find.byType(Hover),
      ),
    );
    expect(selected.selected, isTrue);
    final home = tester.widget<Hover>(
      find.descendant(of: key('sidebar-home'), matching: find.byType(Hover)),
    );
    expect(home.selected, isFalse);
    await tapAndSettle(tester, key('sidebar-home'));
    expect(find.byType(VaultCalendarScreen), findsNothing);
  });

  appTest('boot without Tasks removes the Calendar sidebar entry', (
    tester,
  ) async {
    final fixture = await pumpApp(tester, mobile: false, empty: true);
    expect(key('today-add-task'), findsOneWidget);
    final previous = activeAddonContext;
    final ctx = AddonContext();
    bootAddons(enabledAddons({'tasks'}), ctx);
    activeAddonContext = ctx;
    addTearDown(() {
      activeAddonContext = previous;
      ctx.noteTypes.dispose();
    });
    fixture.app.tree.setSidebarCollapsed(true);
    await settle(tester);
    fixture.app.tree.setSidebarCollapsed(false);
    await settle(tester);
    expect(key('sidebar-calendar'), findsNothing);
    expect(key('sidebar-home'), findsOneWidget);
    expect(key('today-add-task'), findsNothing);
  });
}
