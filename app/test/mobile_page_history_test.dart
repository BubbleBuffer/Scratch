import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/blocks/page_link.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/addons/database/database_view.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/mobile_page_history.dart';
import 'package:notes_app/shell/navigation.dart';

import 'harness.dart';
import 'db_harness.dart' as databases;

MobilePageHistoryState history(WidgetTester t) =>
    t.state<MobilePageHistoryState>(find.byType(MobilePageHistoryScreen));
PageEditorState activeEditor(WidgetTester t) => history(t).editor!;
PageStackNavigator pageStack(WidgetTester t) =>
    PageStackScope.of(t.element(find.byType(HomeScreen)));
Future<void> openMobile(
  WidgetTester t,
  String id, {
  bool legacy = false,
}) async {
  final ctx = find.byType(MobilePageHistoryScreen).evaluate().isEmpty
      ? t.element(find.byType(HomeScreen))
      : activeEditor(t).context;
  unawaited(
    openPage(
      ctx,
      id,
      current.pages.entries.where((entry) => entry.value == id).first.key,
      preserveMobileStack: legacy,
    ),
  );
  await settle(t);
}

Future<void> swipe(WidgetTester t, bool back) async {
  // The page header is outside editable text and inner horizontal scrollers.
  await t.dragFrom(Offset(back ? 80 : 295, 50), Offset(back ? 250 : -250, 0));
  await settle(t);
}

Future<void> editCurrent(WidgetTester t, String title, String text) async {
  final ed = activeEditor(t);
  final block = ed.doc!.flat.firstWhere((n) => n.kind == Kinds.paragraph);
  await t.enterText(
    find.byWidgetPredicate(
      (w) => w is EditableText && w.controller == ed.ctl(block),
    ),
    text,
  );
  await t.enterText(
    find.byWidgetPredicate(
      (w) => w is EditableText && w.controller == ed.titleCtl,
    ),
    title,
  );
  FocusManager.instance.primaryFocus?.unfocus();
}

void main() {
  setUpAll(initCore);

  appTest('one mobile route: A B C, swipe B, Android A, forward B, D drops C', (
    t,
  ) async {
    final s = await pumpApp(t, mobile: true);
    final a = s.pages['Recipes']!,
        b = s.pages['Budget']!,
        c = s.pages['Weekly review']!,
        d = s.pages['Garden plan']!;
    await core(t, (v) async {
      for (final pair in [(a, b), (b, c)]) {
        final page = await v.loadPage(pair.$1);
        final linkId = newId();
        await v.apply(pair.$1, [
          BOp(
            op: OpKind.insert,
            id: linkId,
            after: page.blocks.last.id,
            kind: Kinds.linkToPage,
            text: '',
            target: pair.$2,
            marks: const [],
            checked: false,
          ),
          BOp(
            op: OpKind.setTarget,
            id: linkId,
            kind: '',
            text: '',
            target: pair.$2,
            marks: const [],
            checked: false,
          ),
        ]);
      }
    });
    await openMobile(t, a);
    await tapAndSettle(
      t,
      find.byWidgetPredicate((w) => w is PageLinkRow && w.pageId == b),
    );
    await tapAndSettle(
      t,
      find.byWidgetPredicate((w) => w is PageLinkRow && w.pageId == c),
    );
    expect(pageStack(t).pages, hasLength(1));
    expect(history(t).history.entries.map((e) => e.id), [a, b, c]);
    await editCurrent(t, 'C saved on swipe', 'C block saved on swipe');
    await swipe(t, true);
    expect(activeEditor(t).widget.pageId, b);
    final savedC = await pageOf(t, c);
    expect(savedC.title, 'C saved on swipe');
    expect(savedC.blocks.first.text, 'C block saved on swipe');
    expect(pageStack(t).topId, b);
    expect(s.app.ui.lastPage, b);
    expect(s.app.tree.visited.first, b);
    await editCurrent(
      t,
      'B saved on Android back',
      'B block saved on Android back',
    );
    await t.binding.handlePopRoute();
    await settle(t);
    expect(activeEditor(t).widget.pageId, a);
    final savedB = await pageOf(t, b);
    expect(savedB.title, 'B saved on Android back');
    expect(savedB.blocks.first.text, 'B block saved on Android back');
    await swipe(t, false);
    expect(activeEditor(t).widget.pageId, b);
    await openMobile(t, d, legacy: true);
    expect(history(t).history.entries.map((e) => e.id), [a, b, d]);
    expect(history(t).history.canForward, isFalse);
    expect(s.app.ui.lastPage, d);
    await tapAndSettle(t, find.byKey(const ValueKey('back')));
    expect(activeEditor(t).widget.pageId, b);
    await t.binding.handlePopRoute();
    await settle(t);
    expect(activeEditor(t).widget.pageId, a);
    await swipe(t, true);
    expect(find.byType(MobilePageHistoryScreen), findsNothing);
    expect(pageStack(t).pages, isEmpty);
    expect(s.app.ui.lastPage, isNull);
  });

  appTest('history departure saves title and blocks to the real core', (
    t,
  ) async {
    final s = await pumpApp(t, mobile: true);
    final a = s.pages['Recipes']!, b = s.pages['Budget']!;
    await openMobile(t, a);
    final ed = activeEditor(t);
    final block = ed.doc!.flat.first;
    await t.enterText(
      find.byWidgetPredicate(
        (w) => w is EditableText && w.controller == ed.ctl(block),
      ),
      'Edited before leaving',
    );
    ed.titleCtl.text = 'A saved title';
    FocusManager.instance.primaryFocus?.unfocus();
    await openMobile(t, b);
    final saved = await pageOf(t, a);
    expect(saved.title, 'A saved title');
    expect(saved.blocks.first.text, 'Edited before leaving');
    expect(s.app.pageStore.session(a), isNull);
    await swipe(t, true);
    expect(activeEditor(t).titleCtl.text, 'A saved title');
    expect(activeEditor(t).doc!.flat.first.text, 'Edited before leaving');
  });

  appTest(
    'cancelled drag keeps current editor/session and preview has no session',
    (t) async {
      final s = await pumpApp(t, mobile: true);
      final a = s.pages['Recipes']!, b = s.pages['Budget']!;
      await openMobile(t, a);
      await openMobile(t, b);
      final ed = activeEditor(t);
      final session = ed.session;
      final previousLastSave = s.app.pageStore.lastSave;
      final gesture = await t.startGesture(const Offset(80, 50));
      await gesture.moveBy(const Offset(20, 0));
      await gesture.moveBy(const Offset(60, 0));
      await t.pump();
      expect(find.byType(PageScreen), findsNWidgets(2));
      expect(s.app.pageStore.session(a), isNull);
      expect(s.app.pageStore.session(b), same(session));
      final sheet = t.widget<Transform>(
        find.byKey(const ValueKey('mobile-history-sheet')),
      );
      expect(sheet.transform.storage[12], greaterThan(0));
      await gesture.cancel();
      await settle(t);
      expect(activeEditor(t), same(ed));
      expect(s.app.pageStore.lastSave, same(previousLastSave));
      expect(find.byType(PageScreen), findsOneWidget);
      expect(s.app.ui.lastPage, b);
      await swipe(t, true);
      expect(activeEditor(t).widget.pageId, a);
    },
  );

  appTest(
    'Home entry replaces the history and Android first back reaches Home',
    (t) async {
      final s = await pumpApp(t, mobile: true);
      await openMobile(t, s.pages['Recipes']!);
      await openMobile(t, s.pages['Budget']!);
      final d = s.pages['Garden plan']!;
      unawaited(openPage(t.element(find.byType(HomeScreen)), d, 'Garden plan'));
      await settle(t);
      expect(history(t).history.entries.map((e) => e.id), [d]);
      expect(pageStack(t).pages, hasLength(1));
      await t.binding.handlePopRoute();
      await settle(t);
      expect(find.byType(MobilePageHistoryScreen), findsNothing);
      expect(s.app.ui.lastPage, isNull);
    },
  );

  appTest(
    'pending save blocks input and Android back until the move finishes',
    (t) async {
      final s = await pumpApp(t, mobile: true);
      final a = s.pages['Recipes']!, b = s.pages['Budget']!;
      await openMobile(t, a);
      final host = history(t), editor = activeEditor(t);
      editor.titleFocus.requestFocus();
      await t.pump();
      final gate = Completer<void>();
      final oldSave = s.app.pageStore.lastSave;
      s.app.pageStore.lastSave = gate.future.then((_) => oldSave);
      final opening = host.append(MobilePageEntry(b, 'Budget'));
      try {
        await t.pump();
        expect(editor.titleFocus.hasFocus, isFalse);
        expect(editor.titleFocus.canRequestFocus, isFalse);
        await t.binding.handlePopRoute();
        await t.pump();
        expect(history(t), same(host));
        expect(host.history.current.id, a);
      } finally {
        gate.complete();
      }
      await settle(t);
      await opening;
      expect(host.history.current.id, b);
      expect(pageStack(t).pages, hasLength(1));
    },
  );

  appTest(
    'selection blocks page swipes and reduced motion still follows the finger',
    (t) async {
      t.platformDispatcher.accessibilityFeaturesTestValue =
          const FakeAccessibilityFeatures(disableAnimations: true);
      final s = await pumpApp(t, mobile: true);
      final a = s.pages['Recipes']!, b = s.pages['Budget']!;
      await openMobile(t, a);
      await openMobile(t, b);
      final ed = activeEditor(t);
      ed.titleFocus.requestFocus();
      await t.pump();
      ed.titleCtl.selection = const TextSelection(
        baseOffset: 0,
        extentOffset: 2,
      );
      await swipe(t, true);
      expect(activeEditor(t), same(ed));
      FocusManager.instance.primaryFocus?.unfocus();
      await t.pump();
      final gesture = await t.startGesture(const Offset(80, 50));
      await gesture.moveBy(const Offset(20, 0));
      await gesture.moveBy(const Offset(220, 0));
      await t.pump();
      expect(activeEditor(t), same(ed));
      expect(
        t
            .widget<Transform>(
              find.byKey(const ValueKey('mobile-history-sheet')),
            )
            .transform
            .storage[12],
        greaterThan(150),
      );
      await gesture.up();
      await settle(t);
      expect(activeEditor(t).widget.pageId, a);
    },
  );

  test('history is capped at 50 and append discards the forward suffix', () {
    final h = MobilePageHistory(MobilePageEntry('0', '0'));
    for (var i = 1; i <= 60; i++) {
      h.append(MobilePageEntry('$i', '$i'));
    }
    expect(h.entries, hasLength(50));
    expect(h.entries.first.id, '11');
    expect(h.index, 49);
    h.index -= 2;
    h.append(MobilePageEntry('new', 'new'));
    expect(h.entries, hasLength(49));
    expect(h.current.id, 'new');
    expect(h.entries.any((e) => e.id == '59' || e.id == '60'), isFalse);
  });

  databases.dbTest('real table horizontal scrolling wins over page history', (
    t,
  ) async {
    await databases.pumpDb(t, mobile: true);
    await databases.openView(t, 'Table', mobile: true);
    final host = history(t);
    final before = host.history.current.id;
    // The table sits in the page's view block; its rows scroll sideways.
    final firstRow = find.byKey(ValueKey('row-${databases.projectRows.first}'));
    final horizontal = find.ancestor(
      of: firstRow,
      matching: find.byWidgetPredicate(
        (w) =>
            w is Scrollable &&
            (w.axisDirection == AxisDirection.left ||
                w.axisDirection == AxisDirection.right),
      ),
    );
    final scroller = t.state<ScrollableState>(horizontal.first);
    final offset = scroller.position.pixels;
    await t.dragFrom(
      databases.visibleCenter(t, firstRow),
      const Offset(-220, 0),
    );
    await databases.settle(t);
    expect(scroller.position.pixels, greaterThan(offset));
    expect(host.history.current.id, before);
    expect(host.history.entries, hasLength(1));
    // A database revealed under another page keeps its actual table, without
    // acquiring a second editing session or saving anything on cancellation.
    final row = await databases.rowNamed(t, databases.projectRows.first);
    host.editor!.openPage(row.id);
    await databases.settle(t);
    final active = host.editor!;
    final session = active.session;
    final pageStore = databases.db.app.pageStore;
    final lastSave = pageStore.lastSave;
    final previous = await databases.core(t, (v) => v.loadPage(before));
    final gesture = await t.startGesture(const Offset(80, 50));
    await gesture.moveBy(const Offset(20, 0));
    await gesture.moveBy(const Offset(60, 0));
    await databases.settle(t);
    expect(find.byType(DatabaseView), findsOneWidget);
    expect(firstRow, findsOneWidget);
    expect(pageStore.session(before), isNull);
    expect(pageStore.session(row.id), same(session));
    await gesture.cancel();
    await databases.settle(t);
    expect(host.editor, same(active));
    expect(pageStore.lastSave, same(lastSave));
    expect(find.byType(DatabaseView), findsNothing);
    final after = await databases.core(t, (v) => v.loadPage(before));
    expect(after.updatedMs, previous.updatedMs);
  });
}
