// An untitled page goes by the first line of its body everywhere pages are
// listed, open or not: the rail, Today's All pages disclosure, a breadcrumb ancestor
// and the [[ picker. The stand-in is tertiary italics; with no body text
// it stays "Untitled".

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart' as nav;
import 'package:notes_app/state/app_state.dart';
import 'package:notes_app/state/tree_store.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';

Finder key(String k) => find.byKey(ValueKey(k));

const line = 'Groceries for Sunday';

void expectStandIn(WidgetTester tester, Finder where) {
  final texts = find.descendant(of: where, matching: find.text(line));
  expect(texts, findsOneWidget);
  final t = tester.widget<Text>(texts);
  expect(t.style!.fontStyle, FontStyle.italic);
  expect(
    find.descendant(of: where, matching: find.text('Untitled')),
    findsNothing,
  );
}

void main() {
  setUpAll(initCore);

  appTest('an untitled page that is not open goes by its first line', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final untitled = await core(tester, (v) async {
      final id = await v.createPage();
      await v.apply(id, [
        BOp(
          op: OpKind.insert,
          id: newId(),
          kind: Kinds.paragraph,
          text: '\n  $line\nsecond line',
          marks: const [],
          checked: false,
        ),
      ]);
      return id;
    });
    final blank = await core(tester, (v) => v.createPage());
    final child = await core(
      tester,
      (v) => v.createPage(parent: untitled, title: 'Shop list'),
    );
    final home = tester.element(find.byType(HomeScreen));
    home.read<TreeStore>()
      ..visit(untitled)
      ..expand(untitled);
    await settleUntil(
      tester,
      () => key('tree-$untitled').evaluate().isNotEmpty,
    );

    // The rail and All pages disclosure, without the page ever opening.
    expectStandIn(tester, key('tree-$untitled'));
    await expandAllPages(tester);
    final pageBrowser = find.byWidgetPredicate(
      (widget) =>
          widget is CustomScrollView && widget.scrollDirection == Axis.vertical,
    );
    final browserLabel = find.descendant(
      of: pageBrowser,
      matching: find.text(line),
    );
    expectStandIn(
      tester,
      find.ancestor(of: browserLabel, matching: find.byType(Hover)).first,
    );
    expect(
      tester.widget<Text>(browserLabel).style!.color,
      tester.element(browserLabel).colors.textTertiary,
    );
    // A page without body text keeps "Untitled", upright.
    final none = find.descendant(
      of: key('tree-$blank'),
      matching: find.text('Untitled'),
    );
    expect(none, findsOneWidget);
    expect(tester.widget<Text>(none).style!.fontStyle, isNot(FontStyle.italic));

    // A breadcrumb ancestor.
    final opened = nav.openPage(home, child, 'Shop list');
    await settle(tester);
    expectStandIn(tester, key('crumb-$untitled'));

    // The [[ picker.
    await tapAndSettle(tester, key('start-blank'));
    await typeText(tester, '[[Groceries');
    await settleUntil(
      tester,
      () => key('mention-page-$untitled').evaluate().isNotEmpty,
    );
    expectStandIn(tester, key('mention-page-$untitled'));

    nav.goHome(tester.element(find.byType(HomeScreen, skipOffstage: false)));
    await settle(tester);
    await opened;
  });

  appTest('[[ inserts the first line of an untitled page that is not open', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final untitled = await untitledPage(tester);
    final (host, block) = await hostPage(tester, 'Plan');
    final opened = nav.openPage(
      tester.element(find.byType(HomeScreen)),
      host,
      'Shop list',
    );
    await settleUntil(
      tester,
      () => editableStarting('Plan').evaluate().isNotEmpty,
    );

    final b = editableStarting('Plan');
    await focusAt(
      tester,
      b,
      editState(tester, b).widget.controller.text.length,
    );
    await typeText(tester, ' [[Groc');
    await settleUntil(
      tester,
      () => key('mention-page-$untitled').evaluate().isNotEmpty,
    );
    await tapAndSettle(tester, key('mention-page-$untitled'));

    await expectInserted(tester, host, block, untitled);
    await closePage(tester, opened);
  });

  appTest('@ inserts the first line of an untitled page that is not open', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final untitled = await untitledPage(tester);
    final (host, block) = await hostPage(tester, 'Plan');
    final opened = nav.openPage(
      tester.element(find.byType(HomeScreen)),
      host,
      'Shop list',
    );
    await settleUntil(
      tester,
      () => editableStarting('Plan').evaluate().isNotEmpty,
    );

    final b = editableStarting('Plan');
    await focusAt(
      tester,
      b,
      editState(tester, b).widget.controller.text.length,
    );
    await typeText(tester, ' @Groc');
    await settleUntil(
      tester,
      () => key('mention-page-$untitled').evaluate().isNotEmpty,
    );
    await tapAndSettle(tester, key('mention-page-$untitled'));

    await expectInserted(tester, host, block, untitled);
    await closePage(tester, opened);
  });

  appTest('Link to page lists an untitled page by its first line', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final untitled = await untitledPage(tester);
    final (host, _) = await hostPage(tester, 'Plan');
    final opened = nav.openPage(
      tester.element(find.byType(HomeScreen)),
      host,
      'Shop list',
    );
    await settleUntil(
      tester,
      () => editableStarting('Plan').evaluate().isNotEmpty,
    );

    final b = editableStarting('Plan');
    await focusAt(
      tester,
      b,
      editState(tester, b).widget.controller.text.length,
    );
    // The slash menu opens in an empty block: Enter first, as pages_test does.
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await settle(tester);
    await typeText(tester, '/link');
    await tapAndSettle(tester, key('slash-${Kinds.linkToPage}'));
    await tester.enterText(key('page-picker-field'), 'Groc');
    await settleUntil(
      tester,
      () => key('page-pick-$untitled').evaluate().isNotEmpty,
    );
    // The picker row, for a page that is not open.
    expectStandIn(tester, key('page-pick-$untitled'));
    final label = find.descendant(
      of: key('page-pick-$untitled'),
      matching: find.text(line),
    );
    expect(
      tester.widget<Text>(label).style!.color,
      tester.element(label).colors.textTertiary,
    );
    await tapAndSettle(tester, key('page-pick-$untitled'));

    final links = (await blocksOf(
      tester,
      host,
    )).where((x) => x.kind == Kinds.linkToPage).toList();
    expect(links.map((x) => x.target), [untitled]);
    await closePage(tester, opened);
  });

  appTest(
    'the mobile title bar shows an untitled page\'s first line in italics',
    (tester) async {
      await pumpApp(tester, mobile: true);
      final untitled = await untitledPage(tester);
      final opened = nav.openPage(
        tester.element(find.byType(HomeScreen)),
        untitled,
        '',
      );
      await settleUntil(
        tester,
        () => key('mobile-title').evaluate().isNotEmpty,
      );

      expectBarLabel(tester, line, standIn: true);
      await closePage(tester, opened);
    },
  );

  appTest('the mobile title bar keeps "Untitled" upright for a blank page', (
    tester,
  ) async {
    await pumpApp(tester, mobile: true);
    final blank = await core(tester, (v) => v.createPage());
    final opened = nav.openPage(
      tester.element(find.byType(HomeScreen)),
      blank,
      '',
    );
    await settleUntil(tester, () => key('mobile-title').evaluate().isNotEmpty);

    expectBarLabel(tester, 'Untitled', standIn: false);
    await closePage(tester, opened);
  });
}

/// An untitled page whose body starts with [line]; returns its id.
Future<String> untitledPage(WidgetTester tester) => core(tester, (v) async {
  final id = await v.createPage();
  await v.apply(id, [
    BOp(
      op: OpKind.insert,
      id: newId(),
      kind: Kinds.paragraph,
      text: '\n  $line\nsecond line',
      marks: const [],
      checked: false,
    ),
  ]);
  return id;
});

/// A page titled "Shop list" with one paragraph [text]; returns the page id
/// and the paragraph's block id.
Future<(String, String)> hostPage(WidgetTester tester, String text) async {
  final block = newId();
  final id = await core(tester, (v) async {
    final id = await v.createPage(title: 'Shop list');
    await v.apply(id, [
      BOp(
        op: OpKind.insert,
        id: block,
        kind: Kinds.paragraph,
        text: text,
        marks: const [],
        checked: false,
      ),
    ]);
    return id;
  });
  return (id, block);
}

/// The host paragraph reads "Plan" plus the inserted [line], with a page link
/// mark on the inserted text pointing at [untitled].
Future<void> expectInserted(
  WidgetTester tester,
  String host,
  String block,
  String untitled,
) async {
  final blk = await blockById(tester, host, block);
  expect(blk.text, 'Plan $line');
  final m = blk.marks.singleWhere((m) => m.kind == BMarkKind.pageLink);
  expect(m.start, 5);
  expect(m.end, blk.text.length);
  expect(m.url, untitled);
}

/// Closes a page opened by [opened] and waits for the home screen.
Future<void> closePage(WidgetTester tester, Future<void> opened) async {
  nav.goHome(tester.element(find.byType(HomeScreen, skipOffstage: false)));
  await settle(tester);
  await opened;
}

/// The mobile title bar's label: [text], italic tertiary when [standIn].
void expectBarLabel(WidgetTester tester, String text, {required bool standIn}) {
  final t = tester.widget<Text>(key('mobile-title'));
  expect(t.data, text);
  final c = tester.element(key('mobile-title')).colors;
  expect(t.style!.fontStyle == FontStyle.italic, standIn);
  expect(t.style!.color, standIn ? c.textTertiary : c.textPrimary);
}
