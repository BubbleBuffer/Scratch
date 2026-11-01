// Mobile Home at 390x844: Today with the registered destination dock, the page tree
// and agent chat panels that follow the finger, scrim and Android back
// closing the tree first, and gestures that leave inner scrollers alone.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/api.dart'
    show
        TaskComposer,
        VaultCalendarScreen,
        ShortItemCard,
        TaskData,
        saveTask,
        taskDateString;

import 'harness.dart';

Finder key(String name) => find.byKey(ValueKey(name));

/// The harness's phone is 375x812; these tests use the 390x844 the design
/// was checked at.
Future<TestVault> pumpPhone(WidgetTester tester) async {
  final f = await pumpApp(tester, mobile: true);
  tester.view.physicalSize = const Size(390 * 2, 844 * 2);
  await settle(tester);
  return f;
}

/// The tree is on screen (closed, it sits off the left edge).
bool treeShown(WidgetTester tester) {
  final panel = key('mobile-tree-panel');
  return panel.evaluate().isNotEmpty && tester.getRect(panel).right > 1;
}

/// The agent chat covers the screen (closed, it sits off the right edge).
bool agentShown(WidgetTester tester) {
  final panel = key('mobile-agent-panel');
  return panel.evaluate().isNotEmpty && tester.getRect(panel).left < 389;
}

void main() {
  setUpAll(initCore);

  group('mobile home dock', () {
    appTest(
      'dock orders Pages, Today, Calendar and Search with a task create action',
      (tester) async {
        await pumpPhone(tester);
        for (final k in [
          'nav-pages',
          'nav-today',
          'nav-calendar',
          'nav-search',
          'mobile-create',
        ]) {
          expect(key(k), findsOneWidget, reason: k);
        }
        expect(key('nav-home'), findsNothing);
        expect(find.text('Home'), findsNothing);
        expect(find.text('Pages'), findsOneWidget);
        expect(key('new-page'), findsNothing);
        expect(key('nav-pages'), findsOneWidget);
        expect(tester.getRect(key('mobile-dock')).bottom, closeTo(844, 0.5));
        final order = ['nav-pages', 'nav-today', 'nav-calendar', 'nav-search'];
        for (var i = 1; i < order.length; i++) {
          expect(
            tester.getRect(key(order[i])).left,
            greaterThan(tester.getRect(key(order[i - 1])).left),
          );
        }
        expect(
          tester.getRect(key('mobile-create')).bottom,
          lessThan(tester.getRect(key('mobile-dock')).top),
        );
      },
    );

    appTest('the tree button opens the tree; a tap on the scrim closes it', (
      tester,
    ) async {
      await pumpPhone(tester);
      expect(treeShown(tester), isFalse);
      await tapAndSettle(tester, key('nav-pages'));
      expect(treeShown(tester), isTrue);
      expect(key('pages-trash'), findsOneWidget);
      expect(key('pages-settings'), findsOneWidget);
      await tester.tapAt(const Offset(370, 500));
      await settle(tester);
      expect(treeShown(tester), isFalse);
      expect(key('nav-pages'), findsOneWidget);
    });

    appTest('search opens search and Pages create action opens a new page', (
      tester,
    ) async {
      await pumpPhone(tester);
      await tapAndSettle(tester, key('nav-search'));
      expect(key('search-field'), findsOneWidget);
      await tester.binding.handlePopRoute();
      await settle(tester);
      expect(key('search-field'), findsNothing);
      expect(key('nav-search'), findsOneWidget);

      await tapAndSettle(tester, key('nav-pages'));
      await tapAndSettle(tester, key('new-page'));
      expect(key('page-title'), findsOneWidget);
      await tester.binding.handlePopRoute();
      await settle(tester);
      expect(key('page-title'), findsNothing);
      expect(key('nav-pages'), findsOneWidget);
    });

    appTest('Today and Calendar create tasks on the shared selected date', (
      tester,
    ) async {
      final f = await pumpPhone(tester);
      final selected = DateTime(2026, 11, 4);
      f.app.agenda.selectDay(selected);
      await settle(tester);
      await tapAndSettle(tester, key('mobile-create'));
      expect(
        tester.widget<TaskComposer>(find.byType(TaskComposer)).initialDate,
        selected,
      );
      await tester.binding.handlePopRoute();
      await settle(tester);
      await tapAndSettle(tester, key('nav-calendar'));
      expect(find.byType(VaultCalendarScreen), findsOneWidget);
      for (final name in [
        'nav-pages',
        'nav-today',
        'nav-calendar',
        'nav-search',
      ]) {
        expect(key(name), findsOneWidget);
      }
      await tapAndSettle(tester, key('mobile-create'));
      expect(
        tester.widget<TaskComposer>(find.byType(TaskComposer)).initialDate,
        selected,
      );
      await tester.binding.handlePopRoute();
      await settle(tester);
      await tapAndSettle(tester, key('nav-pages'));
      expect(find.byType(VaultCalendarScreen), findsNothing);
      expect(treeShown(tester), isTrue);
      await tester.tapAt(const Offset(370, 500));
      await settle(tester);
      await tapAndSettle(tester, key('nav-calendar'));
      await tapAndSettle(tester, key('nav-today'));
      expect(find.byType(VaultCalendarScreen), findsNothing);
      expect(f.app.agenda.selectedDay, selected);
    });

    appTest(
      'last phone Calendar item scrolls above the floating create action',
      (tester) async {
        final f = await pumpPhone(tester);
        final selected = f.app.agenda.selectedDay;
        await core(tester, (vault) async {
          for (var i = 0; i < 12; i++) {
            final page = await vault.createPage(
              title: 'Agenda item $i',
              pageType: 'task',
            );
            await saveTask(
              vault,
              page,
              TaskData(date: taskDateString(selected), calendar: 'personal'),
            );
          }
          await vault.flush();
        });
        await tapAndSettle(tester, key('nav-calendar'));
        await tapAndSettle(tester, find.text('Day'));
        final agenda = find.descendant(
          of: find.byType(VaultCalendarScreen),
          matching: find.byType(SingleChildScrollView),
        );
        await tester.drag(agenda, const Offset(0, -2000));
        await settle(tester);
        final lastCard = find.byType(ShortItemCard).last;
        expect(
          tester.getRect(lastCard).bottom,
          lessThan(tester.getRect(key('mobile-create')).top),
        );
        expect(
          tester.getRect(lastCard).top,
          greaterThanOrEqualTo(tester.getRect(agenda).top),
        );
      },
    );
  });

  group('mobile home panels', () {
    appTest('a fling right opens the tree and a fling left closes it', (
      tester,
    ) async {
      await pumpPhone(tester);
      await tester.fling(
        key('today-selected-heading'),
        const Offset(300, 0),
        1000,
      );
      await settle(tester);
      expect(treeShown(tester), isTrue);
      await tester.fling(key('mobile-tree-panel'), const Offset(-300, 0), 1000);
      await settle(tester);
      expect(treeShown(tester), isFalse);
    });

    appTest(
      'a fling left opens the agent chat full width; a fling right closes it',
      (tester) async {
        await pumpPhone(tester);
        await tester.fling(
          key('today-selected-heading'),
          const Offset(-300, 0),
          1000,
        );
        await settle(tester);
        expect(agentShown(tester), isTrue);
        expect(tester.getRect(key('mobile-agent-panel')).left, closeTo(0, 0.5));
        expect(
          tester.getRect(key('mobile-agent-panel')).width,
          closeTo(390, 0.5),
        );
        expect(key('agent-screen'), findsOneWidget);
        await tester.fling(key('agent-screen'), const Offset(300, 0), 1000);
        await settle(tester);
        expect(agentShown(tester), isFalse);
        await tester.fling(
          key('today-selected-heading'),
          const Offset(-300, 0),
          1000,
        );
        await settle(tester);
        await tester.binding.handlePopRoute();
        await settle(tester);
        expect(agentShown(tester), isFalse);
        expect(key('mobile-dock'), findsOneWidget);
      },
    );

    appTest('the tree follows the finger; a short release returns it closed', (
      tester,
    ) async {
      await pumpPhone(tester);
      final gesture = await tester.startGesture(
        tester.getCenter(key('today-selected-heading')),
      );
      await gesture.moveBy(const Offset(120, 0));
      await tester.pump();
      expect(tester.getRect(key('mobile-tree-panel')).right, closeTo(120, 1));
      await gesture.up();
      await settle(tester);
      expect(treeShown(tester), isFalse);
    });

    appTest('a drag past half the tree opens it on release', (tester) async {
      await pumpPhone(tester);
      final gesture = await tester.startGesture(
        tester.getCenter(key('today-selected-heading')),
      );
      await gesture.moveBy(const Offset(120, 0));
      await gesture.moveBy(const Offset(100, 0));
      await tester.pump();
      await gesture.up();
      await settle(tester);
      expect(treeShown(tester), isTrue);
    });

    appTest('Android back closes the tree before Home', (tester) async {
      await pumpPhone(tester);
      await tapAndSettle(tester, key('nav-pages'));
      expect(treeShown(tester), isTrue);
      await tester.binding.handlePopRoute();
      await settle(tester);
      expect(treeShown(tester), isFalse);
      expect(key('nav-pages'), findsOneWidget);
    });

    appTest('a page opened from the tree closes the panel', (tester) async {
      final f = await pumpPhone(tester);
      await tapAndSettle(tester, key('nav-pages'));
      expect(treeShown(tester), isTrue);
      await tapAndSettle(tester, key('pages-${f.pages['Recipes']}'));
      expect(key('page-title'), findsOneWidget);
      await tester.binding.handlePopRoute();
      await settle(tester);
      expect(key('page-title'), findsNothing);
      expect(treeShown(tester), isFalse);
    });

    appTest(
      'a horizontal drag on the date strip scrolls dates and moves no panel',
      (tester) async {
        await pumpPhone(tester);
        // A previous closing drag must not be replayed if navigation loses
        // the next arena to the date strip.
        await tapAndSettle(tester, key('nav-pages'));
        await tester.fling(
          key('mobile-tree-panel'),
          const Offset(-300, 0),
          1000,
        );
        await settle(tester);
        await tester.ensureVisible(key('today-date-strip'));
        await settle(tester);
        final scroller = tester.state<ScrollableState>(
          find.descendant(
            of: key('today-date-strip'),
            matching: find.byType(Scrollable),
          ),
        );
        final before = scroller.position.pixels;
        final selected = current.app.agenda.selectedDay;
        await tester.fling(
          key('today-date-strip'),
          const Offset(-200, 0),
          1000,
        );
        await settle(tester);
        expect(scroller.position.pixels, greaterThan(before));
        expect(current.app.agenda.selectedDay, selected);
        expect(treeShown(tester), isFalse);
        expect(agentShown(tester), isFalse);
      },
    );

    appTest(
      'a vertical scroll of Today moves no panel and keeps the strip pinned',
      (tester) async {
        await pumpPhone(tester);
        await expandAllPages(tester);
        final body = find.byWidgetPredicate(
          (widget) =>
              widget is CustomScrollView &&
              widget.scrollDirection == Axis.vertical,
        );
        final list = tester.state<ScrollableState>(
          find.descendant(of: body, matching: find.byType(Scrollable)).first,
        );
        // Expanding the page browser scrolls away from Today. Return through
        // the scroll position: the heading can be outside the built viewport
        // once the vault also contains its device record.
        list.position.jumpTo(list.position.minScrollExtent);
        await settle(tester);
        expect(key('today-selected-heading'), findsOneWidget);
        final strip = tester.getRect(key('today-date-strip'));
        final before = list.position.pixels;
        await tester.fling(
          key('today-selected-heading'),
          const Offset(0, -300),
          1000,
        );
        await settle(tester);
        expect(list.position.pixels, greaterThan(before));
        expect(tester.getRect(key('today-date-strip')), strip);
        expect(treeShown(tester), isFalse);
        expect(agentShown(tester), isFalse);
      },
    );

    appTest('reduced motion opens the tree at once', (tester) async {
      tester.platformDispatcher.accessibilityFeaturesTestValue =
          const FakeAccessibilityFeatures(disableAnimations: true);
      addTearDown(
        tester.platformDispatcher.clearAccessibilityFeaturesTestValue,
      );
      await pumpPhone(tester);
      await tester.tap(key('nav-pages'));
      await tester.pump();
      expect(treeShown(tester), isTrue);
    });
  });
}
