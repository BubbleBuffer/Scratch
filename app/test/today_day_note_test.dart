import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/editor/mention_menu.dart' show isoDate;
import 'package:notes_app/state/agenda_store.dart';

import 'harness.dart';

Finder key(String value) => find.byKey(ValueKey(value));
Finder starter() => find.descendant(
  of: key('today-note-start'),
  matching: find.byType(EditableText),
);

Future<String?> dayNote(WidgetTester tester, DateTime day) => core(
  tester,
  (v) => v.run(
    (c) => c.dailyNote(
      dayMs: dayStamp(day),
      title: dayTitle(day, year: true),
      create: false,
    ),
  ),
);

Future<void> selectDay(WidgetTester tester, DateTime day) async {
  current.app.agenda.selectDay(day);
  await settle(tester);
  await settleUntil(
    tester,
    () =>
        starter().evaluate().isNotEmpty ||
        find.byType(PageScreen).evaluate().isNotEmpty,
  );
}

void main() {
  setUpAll(initCore);

  appTest(
    'short desktop note menus accept real taps beyond the document body',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final day = current.app.agenda.selectedDay;
      await settleUntil(tester, () => starter().evaluate().isNotEmpty);
      await tester.ensureVisible(starter());
      await tester.enterText(starter(), 'Small note');
      await settleUntil(
        tester,
        () => editableWith('Small note').evaluate().isNotEmpty,
      );
      final page = (await dayNote(tester, day))!;
      final editor = tester.state<PageEditorState>(find.byType(PageScreen));
      final block = editor.doc!.flat
          .firstWhere((n) => n.text == 'Small note')
          .id;
      await tester.enterText(editableWith('Small note'), '/');
      await settle(tester);
      expect(key('slash-menu'), findsOneWidget);
      expect(
        tester.getSize(key('slash-menu')).height,
        greaterThan(tester.getSize(find.byKey(editor.stackKey)).height),
      );
      expect(key('slash-heading1').hitTestable(), findsOneWidget);
      await tapAndSettle(tester, key('slash-heading1').hitTestable());
      expect((await blockById(tester, page, block)).kind, Kinds.heading1);
      expect(key('slash-menu'), findsNothing);

      await tester.enterText(editableWith(''), 'A little heading');
      final heading = editState(tester, editableWith('A little heading'));
      // Deliver one genuine keyboard selection change. Pre-setting the
      // controller first would make EditableText see an unchanged value and
      // suppress its onSelectionChanged callback on this already-focused row.
      heading.userUpdateTextEditingValue(
        heading.widget.controller.value.copyWith(
          selection: const TextSelection(baseOffset: 0, extentOffset: 1),
        ),
        SelectionChangedCause.keyboard,
      );
      await settle(tester);
      expect(key('selection-toolbar'), findsOneWidget);
      expect(key('fmt-link').hitTestable(), findsOneWidget);
      await tapAndSettle(tester, key('fmt-link').hitTestable());
      final link = find.descendant(
        of: key('link-field'),
        matching: find.byType(EditableText),
      );
      expect(link.hitTestable(), findsOneWidget);
      await tester.tap(link.hitTestable());
      await tester.enterText(link, 'https://example.com/day-note');
      await tester.testTextInput.receiveAction(TextInputAction.search);
      await settle(tester);
      expect(
        (await blockById(tester, page, block)).marks,
        contains(
          predicate<BMark>(
            (mark) =>
                mark.kind == BMarkKind.link &&
                mark.start == 0 &&
                mark.end == 1 &&
                mark.url == 'https://example.com/day-note',
          ),
        ),
      );

      // The block menu uses the same screen overlay and action state too.
      final node = editor.doc!.byId[block]!;
      editor.openBlockMenu(
        node,
        tester.getRect(editableWith('A little heading')),
      );
      await settle(tester);
      expect(key('menu-turn-into').hitTestable(), findsOneWidget);
      await tapAndSettle(tester, key('menu-turn-into').hitTestable());
      expect(key('slash-heading2').hitTestable(), findsOneWidget);
      await tapAndSettle(tester, key('slash-heading2').hitTestable());
      expect((await blockById(tester, page, block)).kind, Kinds.heading2);

      await tester.enterText(editableWith('A little heading'), '');
      await typeText(tester, '@');
      expect(key('mention-menu'), findsOneWidget);
      final today = key('mention-date-${isoDate(day)}');
      expect(today.hitTestable(), findsOneWidget);
      await tapAndSettle(tester, today.hitTestable());
      expect(
        (await blockById(tester, page, block)).marks,
        contains(
          predicate<BMark>(
            (mark) => mark.kind == BMarkKind.date && mark.url == isoDate(day),
          ),
        ),
      );
    },
  );

  for (final mobile in [false, true]) {
    appTest(
      'bootstrap slash opens ordinary block menu on ${mobile ? 'mobile' : 'desktop'}',
      (tester) async {
        await pumpApp(tester, mobile: mobile, empty: true);
        final day = current.app.agenda.selectedDay;
        await settleUntil(tester, () => starter().evaluate().isNotEmpty);
        await tester.ensureVisible(starter());
        await tester.enterText(starter(), '/');
        await settleUntil(
          tester,
          () => key('slash-menu').evaluate().isNotEmpty,
        );
        final page = (await dayNote(tester, day))!;
        final block = (await blocksOf(
          tester,
          page,
        )).firstWhere((b) => b.text == '/').id;
        expect(key('slash-heading1').hitTestable(), findsOneWidget);
        await tapAndSettle(tester, key('slash-heading1').hitTestable());
        expect((await blockById(tester, page, block)).kind, Kinds.heading1);
        expect((await blockById(tester, page, block)).text, isEmpty);
        expect(key('slash-menu'), findsNothing);
      },
    );
  }

  appTest(
    'long inline note keeps mobile toolbar and slash menu above keyboard',
    (tester) async {
      await pumpApp(tester, mobile: true, empty: true);
      final day = current.app.agenda.selectedDay;
      final page = (await core(
        tester,
        (v) => v.run(
          (c) => c.dailyNote(
            dayMs: dayStamp(day),
            title: dayTitle(day, year: true),
            create: true,
          ),
          writes: true,
        ),
      ))!;
      await core(tester, (v) async {
        final tasks = (await v.loadPage(page)).blocks.first;
        String after = tasks.id;
        final ops = <BOp>[];
        for (var i = 0; i < 40; i++) {
          final id = newId();
          ops.add(
            BOp(
              op: OpKind.insert,
              id: id,
              after: after,
              kind: Kinds.paragraph,
              text: 'Long note line $i',
              marks: const [],
              checked: false,
            ),
          );
          after = id;
        }
        await v.apply(page, ops);
      });
      await settleUntil(
        tester,
        () => editableWith('Long note line 0').evaluate().isNotEmpty,
      );
      await tester.ensureVisible(editableWith('Long note line 0'));
      await focusAt(tester, editableWith('Long note line 0'), 0);
      await showKeyboard(tester);
      expect(key('kb-add'), findsOneWidget);
      final context = tester.element(key('kb-add'));
      final mq = MediaQuery.of(context);
      final toolbar = tester.getRect(key('kb-add'));
      expect(toolbar.top, greaterThanOrEqualTo(0));
      expect(
        toolbar.bottom,
        lessThanOrEqualTo(mq.size.height - mq.viewInsets.bottom),
      );
      expect(
        tester.getRect(editableWith('Long note line 39')).top,
        greaterThan(mq.size.height),
      );
      await tester.enterText(editableWith('Long note line 0'), '/');
      await settle(tester);
      expect(key('slash-menu'), findsOneWidget);
      final menu = tester.getRect(key('slash-menu'));
      expect(menu.top, greaterThanOrEqualTo(0));
      expect(menu.bottom, lessThanOrEqualTo(toolbar.top));
      await tapAndSettle(tester, key('kb-hide'));
      expect(key('kb-add'), findsNothing);
    },
  );

  for (final mobile in [false, true]) {
    final form = mobile ? 'mobile' : 'desktop';

    appTest('view and focus do not create a day note on $form', (tester) async {
      await pumpApp(tester, mobile: mobile, empty: true);
      final day = current.app.agenda.selectedDay;
      await settleUntil(tester, () => starter().evaluate().isNotEmpty);
      expect(await dayNote(tester, day), isNull);
      await tester.ensureVisible(starter());
      await tapAndSettle(tester, starter());
      expect(await dayNote(tester, day), isNull);
      expect(find.byType(PageScreen), findsNothing);
      await selectDay(tester, DateTime(2026, 10, 8));
      expect(await dayNote(tester, DateTime(2026, 10, 8)), isNull);
      expect(key('capture-field'), findsOneWidget);
    });

    appTest(
      'first typing creates and then edits the real inline note on $form',
      (tester) async {
        await pumpApp(tester, mobile: mobile, empty: true);
        final day = current.app.agenda.selectedDay;
        await settleUntil(tester, () => starter().evaluate().isNotEmpty);
        await tester.ensureVisible(starter());
        await tester.enterText(starter(), 'Plan the garden');
        await settleUntil(
          tester,
          () => editableWith('Plan the garden').evaluate().isNotEmpty,
        );
        final page = (await dayNote(tester, day))!;
        final editor = tester.state<PageEditorState>(find.byType(PageScreen));
        expect(editor.widget.embedded, isTrue);
        expect(key('page-title'), findsNothing);
        expect(key('page-menu'), findsNothing);
        expect(
          editState(
            tester,
            editableWith('Plan the garden'),
          ).widget.focusNode.hasFocus,
          isTrue,
        );
        final blocks = await blocksOf(tester, page);
        expect(blocks.map((b) => b.text), ['Tasks', 'Plan the garden', 'Log']);
        expect(blocks.first.kind, Kinds.toggle);
        expect(blocks.last.kind, Kinds.toggle);
        expect(editor.isOpen(editor.doc!.byId[blocks.last.id]!), isFalse);

        await focusAt(tester, editableWith('Plan the garden'), 15);
        await typeText(tester, ' tomorrow');
        expect(
          (await blocksOf(tester, page)).map((b) => b.text),
          contains('Plan the garden tomorrow'),
        );
        expect(starter(), findsNothing);
        expect(key('capture-field'), findsOneWidget);

        // Opening the document releases the inline session and preserves edits.
        await tester.ensureVisible(key('today-note-open-page'));
        await tapAndSettle(tester, key('today-note-open-page'));
        expect(find.byType(PageScreen), findsOneWidget);
        expect(
          tester.widget<PageScreen>(find.byType(PageScreen)).embedded,
          isFalse,
        );
        await tester.enterText(
          editableWith('Plan the garden tomorrow'),
          'Plan the garden tomorrow morning',
        );
        await tapAndSettle(tester, key('back'));
        await settleUntil(
          tester,
          () => find.byType(PageScreen).evaluate().isNotEmpty,
        );
        expect(
          tester.widget<PageScreen>(find.byType(PageScreen)).embedded,
          isTrue,
        );
        expect(
          editableWith('Plan the garden tomorrow morning'),
          findsOneWidget,
        );
        expect(
          (await blocksOf(tester, page)).map((b) => b.text),
          contains('Plan the garden tomorrow morning'),
        );
      },
    );

    for (final day in [DateTime(2026, 10, 4), DateTime(2026, 10, 9)]) {
      appTest('existing ${isoDate(day)} note is editable inline on $form', (
        tester,
      ) async {
        await pumpApp(tester, mobile: mobile, empty: true);
        final page = (await core(
          tester,
          (v) => v.run(
            (c) => c.dailyNote(
              dayMs: dayStamp(day),
              title: dayTitle(day, year: true),
              create: true,
            ),
            writes: true,
          ),
        ))!;
        await core(tester, (v) async {
          final data = await v.loadPage(page);
          final tasks = data.blocks.firstWhere((b) => b.text == 'Tasks');
          final log = data.blocks.firstWhere((b) => b.text == 'Log');
          await v.apply(page, [
            BOp(
              op: OpKind.insert,
              id: newId(),
              after: tasks.id,
              kind: Kinds.paragraph,
              text: 'Existing day content',
              marks: const [],
              checked: false,
            ),
            BOp(
              op: OpKind.insert,
              id: newId(),
              parent: log.id,
              kind: Kinds.paragraph,
              text: 'Hidden log entry',
              marks: const [],
              checked: false,
            ),
          ]);
        });
        await selectDay(tester, day);
        expect(starter(), findsNothing);
        expect(editableWith('Existing day content'), findsOneWidget);
        expect(editableWith('Hidden log entry'), findsNothing);
        await tester.ensureVisible(editableWith('Existing day content'));
        await tester.enterText(
          editableWith('Existing day content'),
          'Edited selected day',
        );
        await settle(tester);
        expect(
          (await blocksOf(tester, page)).map((b) => b.text),
          contains('Edited selected day'),
        );
        expect(await dayNote(tester, day), page);
        expect(await dayNote(tester, DateTime(2026, 10, 6)), isNull);
      });
    }

    appTest(
      'first edit on a future day stays with that day when switching on $form',
      (tester) async {
        await pumpApp(tester, mobile: mobile, empty: true);
        final day = DateTime(2026, 10, 9);
        final next = DateTime(2026, 10, 10);
        await selectDay(tester, day);
        await tester.ensureVisible(starter());
        await tester.enterText(starter(), 'Future draft');
        current.app.agenda.selectDay(next);
        await tester.pump();
        await settle(tester);
        await settleUntil(tester, () => starter().evaluate().isNotEmpty);
        expect(current.app.agenda.selectedDay, next);
        expect(await dayNote(tester, next), isNull);
        expect(tester.widget<EditableText>(starter()).controller.text, isEmpty);
        final page = (await dayNote(tester, day))!;
        expect(
          (await blocksOf(tester, page)).map((b) => b.text),
          contains('Future draft'),
        );
        await selectDay(tester, day);
        expect(editableWith('Future draft'), findsOneWidget);
        await focusAt(tester, editableWith('Future draft'), 12);
        await tester.sendKeyEvent(LogicalKeyboardKey.enter);
        await settle(tester);
        expect((await blocksOf(tester, page)).length, 4);
      },
    );
  }
}
