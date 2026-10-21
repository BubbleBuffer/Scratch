import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/model.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/editor/rich_controller.dart';
import 'package:notes_app/editor/todo_dates.dart';
import 'package:notes_app/addons/database/agenda_settings.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;
import 'package:notes_app/ui/ui.dart';

import 'db_harness.dart' as d;
import 'harness.dart';
import 'today_editor_fixture.dart';

void main() {
  setUpAll(initCore);
  group('to-do date labels and text', () {
    setUp(() => Clock.fixedAt = fixedNow);
    tearDown(() => Clock.fixedAt = null);
    test('relative labels use calendar days and year, rejecting invalid ISO dates', () {
      expect(todoDateLabel(DateTime(2026, 10, 6)), 'Today');
      expect(todoDateLabel(DateTime(2026, 10, 7)), 'Tomorrow');
      expect(todoDateLabel(DateTime(2026, 10, 5)), 'Yesterday');
      expect(todoDateLabel(DateTime(2026, 10, 8)), 'Thursday');
      expect(todoDateLabel(DateTime(2026, 10, 13)), '13 Oct');
      expect(todoDateLabel(DateTime(2027, 1, 8)), '8 Jan 2027');
      expect(todoDate('2026-02-30'), isNull);
      expect(todoDate('2026-10-06'), isNotNull);
    });
    test('append, replace and remove preserve other marks and UTF-16 text', () {
      final n = Node(id: 'task', kind: Kinds.todo, text: '🌱 Plant garlic',
          marks: [const BMark(start: 3, end: 8, kind: BMarkKind.bold, url: '')]);
      final first = replaceTodoDate(n, DateTime(2026, 10, 6));
      expect(first.text, '🌱 Plant garlic @October 6, 2026');
      expect(first.marks.first, n.marks.first);
      n.text = first.text; n.marks = first.marks;
      final next = replaceTodoDate(n, DateTime(2026, 10, 7));
      expect(next.text, '🌱 Plant garlic @October 7, 2026');
      expect(next.marks.where((m) => m.kind == BMarkKind.date).single.url, '2026-10-07');
      n.text = next.text; n.marks = next.marks;
      final removed = replaceTodoDate(n, null);
      expect(removed.text, '🌱 Plant garlic');
      expect(removed.marks, [const BMark(start: 3, end: 8, kind: BMarkKind.bold, url: '')]);
    });
    test('Auto mirrors Date plus completion property and excludes system times', () {
      PropDef prop(PropKind kind) => PropDef(key: kind.name, name: kind.name, kind: kind, options: const [], config: '');
      expect(resolvedAgendaRole([prop(PropKind.createdTime), prop(PropKind.checkbox)], AgendaRole.auto), AgendaRole.off);
      expect(resolvedAgendaRole([prop(PropKind.date)], AgendaRole.auto), AgendaRole.events);
      expect(resolvedAgendaRole([prop(PropKind.date), prop(PropKind.checkbox)], AgendaRole.auto), AgendaRole.tasks);
      expect(resolvedAgendaRole([prop(PropKind.date), prop(PropKind.status)], AgendaRole.auto), AgendaRole.tasks);
      expect(resolvedAgendaRole([prop(PropKind.date)], AgendaRole.off), AgendaRole.off);
    });
  });

  for (final mobile in [false, true]) {
    final platform = mobile ? 'mobile' : 'desktop';
    appTest('Set date and Remove date persist a to-do mark on $platform', (tester) async {
      await pumpApp(tester, mobile: mobile);
      final page = await openTodoDateFixture(tester);
      final todo = editableStarting('Plant garlic');
      final editor = tester.state<PageEditorState>(find.byType(PageScreen));
      final node = editor.doc!.flat.firstWhere((n) => n.text.startsWith('Plant garlic'));
      Future<void> menu() async {
        if (mobile) {
          await tester.longPress(todo);
          await settle(tester);
          await tapAndSettle(tester, find.byKey(const ValueKey('sheet-set-date')));
        } else {
          editor.openBlockMenu(node, tester.getRect(todo));
          await settle(tester);
          await tapAndSettle(tester, find.byKey(const ValueKey('menu-set-date')));
        }
      }
      await menu();
      expect(find.text('Remove date'), findsOneWidget);
      expect(find.byKey(const ValueKey('date-time-toggle')), findsNothing);
      await tapAndSettle(tester, find.byKey(const ValueKey('day-2026-10-8')));
      final saved = await blockById(tester, page, node.id);
      expect(saved.text, 'Plant garlic @October 8, 2026');
      expect(saved.marks.where((m) => m.kind == BMarkKind.date).single.url, '2026-10-08');
      final controller = editState(tester, todo).widget.controller as RichController;
      expect(controller.dateChips, isTrue);
      expect(controller.buildTextSpan(context: tester.element(todo), style: ts(TypeScale.desktop.body, AppColors.light.textPrimary), withComposing: false).children!
          .whereType<TextSpan>().map((s) => s.semanticsLabel), contains('Thursday'));
      await focusAt(tester, todo, 5);
      expect(controller.dateChips, isFalse);
      expect(controller.selection.baseOffset, 5);
      FocusManager.instance.primaryFocus?.unfocus();
      await settle(tester);
      await menu();
      await tapAndSettle(tester, find.byKey(const ValueKey('date-clear')));
      expect((await blockById(tester, page, node.id)).text, 'Plant garlic');
    });

    appTest('Colour menu persists a top-level space colour on $platform', (tester) async {
      await pumpApp(tester, mobile: mobile, persist: true);
      final page = await openTodoDateFixture(tester);
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu')));
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu-colour')));
      for (final name in spaceColorNames) {
        expect(find.byKey(ValueKey('page-colour-$name')), findsOneWidget);
      }
      await tapAndSettle(tester, find.byKey(const ValueKey('page-colour-moss')));
      expect((await pageOf(tester, page)).color, 'moss');
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu-colour')));
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu-colour-none')));
      expect((await pageOf(tester, page)).color, 'none');
      expect(spaceColor('none', Brightness.light), isNull);
      expect(spaceColor('none', Brightness.dark), isNull);
      Navigator.of(tester.element(find.byKey(const ValueKey('page-menu-colour')))).pop();
      await settle(tester);

      // Reopen the vault and run the same backfill used by boot: explicit
      // None must remain different from a legacy missing colour.
      await restartApp(tester, restoreLastPage: false);
      await core(tester, (v) => v.run((c) => c.ensureSpaceColors(), writes: true));
      await settle(tester);
      expect((await pageOf(tester, page)).color, 'none');
      if (!mobile) {
        final sidebarDot = find.byKey(ValueKey('rail-space-tile-$page'));
        expect(sidebarDot, findsNothing);
      }
      nav.openPage(tester.element(find.byType(HomeScreen)), page, 'Garden plan');
      await settle(tester);
      await tapAndSettle(tester, find.byKey(const ValueKey('page-menu')));
      final dot = find.descendant(of: find.byKey(const ValueKey('page-menu-colour')), matching: find.byType(SpaceDot));
      expect(dot, findsOneWidget);
      expect(find.descendant(of: dot, matching: find.byType(Container)), findsNothing);
    });

    d.dbTest('Show in Today Off removes database agenda items on $platform', (tester) async {
      await d.pumpDb(tester, mobile: mobile);
      final monday = DateTime(2026, 10, 5), nextMonday = DateTime(2026, 10, 12);
      Future<List<AgendaItem>> items() => d.core(tester, (v) => v.run((c) => c.agenda(
        timedFromMs: monday.millisecondsSinceEpoch, timedToMs: nextMonday.millisecondsSinceEpoch,
        dayFromMs: DateTime.utc(2026, 10, 5).millisecondsSinceEpoch, dayToMs: DateTime.utc(2026, 10, 12).millisecondsSinceEpoch)));
      expect((await items()).where((i) => i.collection == d.db.projects), isNotEmpty);
      await d.openView(tester, 'Table', mobile: mobile);
      await d.tapKey(tester, 'db-settings');
      await d.tapKey(tester, 'settings-agenda');
      expect(find.text('Auto · Tasks'), findsOneWidget);
      for (final property in d.db.schema.values.where((p) => p.kind == PropKind.createdTime || p.kind == PropKind.updatedTime)) {
        expect(d.byKey('agenda-date-${property.key}'), findsNothing);
      }
      await d.tapKey(tester, 'agenda-role-off');
      final settings = await d.core(tester, (v) => v.run((c) => c.agendaSettings(collection: d.db.projects)));
      expect(settings.role, AgendaRole.off);
      expect((await items()).where((i) => i.collection == d.db.projects), isEmpty);
    });
  }
}
