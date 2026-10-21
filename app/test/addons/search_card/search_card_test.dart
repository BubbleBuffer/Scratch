// The search card on the real core: a query block shows its live results,
// ticking a task checks it (the row stays while the query still matches it),
// the edit mode applies or cancels, and a card releases its watch when it goes
// away or its query changes. The tick-and-leave message and its Undo are unit
// tested in query_ticks_test.dart: the core cannot drop a done task by status.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/search_card/query_card.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/ui/buttons.dart';

import 'search_card_fixture.dart';

void main() {
  setUpAll(initCore);

  appTest('a query block shows the pages that match it', (tester) async {
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Quarterly review');
    await seedPage(tester, 'Quarterly budget');
    await seedPage(tester, 'Garden plan');
    final block = await openDashboard(tester, query: 'quarterly');

    await settleUntil(
      tester,
      () => find
          .descendant(of: inCard(block), matching: find.text('Quarterly review'))
          .evaluate()
          .isNotEmpty,
    );
    final card = inCard(block);
    expect(
      find.descendant(of: card, matching: find.text('Quarterly budget')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: card, matching: find.text('Garden plan')),
      findsNothing,
    );
    expect(
      find.descendant(of: card, matching: find.text('2')),
      findsOneWidget,
    );
  });

  appTest('editing a matching page elsewhere updates the card live', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final plans = await seedPage(tester, 'Plans', body: 'Nothing here yet');
    final block = await openDashboard(tester, query: 'quarterly');
    expect(
      find.descendant(of: inCard(block), matching: find.text('Plans')),
      findsNothing,
    );

    await core(
      tester,
      (v) => v.apply(plans, [
        BOp(
          op: OpKind.insert,
          id: newId(),
          kind: 'paragraph',
          text: 'Quarterly targets',
          marks: const [],
          checked: false,
        ),
      ]),
    );
    await settleUntil(
      tester,
      () => find
          .descendant(of: inCard(block), matching: find.text('Plans'))
          .evaluate()
          .isNotEmpty,
    );
    expect(
      find.descendant(of: inCard(block), matching: find.text('Plans')),
      findsOneWidget,
    );
  });

  appTest('ticking a task checks it, and it stays while the query matches it', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final task = await seedPage(tester, 'Ship release', pageType: taskPageType);
    await makeTask(tester, task);
    final block = await openDashboard(tester, query: 'release');
    final card = inCard(block);
    final checked = find.descendant(
      of: card,
      matching: find.byWidgetPredicate((w) => w is StacksCheckbox && w.checked),
    );
    final unchecked = find.descendant(
      of: card,
      matching: find.byWidgetPredicate((w) => w is StacksCheckbox && !w.checked),
    );
    expect(unchecked, findsOneWidget);

    await tester.tap(unchecked);
    // This query does not name the status, so the done task still matches
    // "release": the row comes back checked, and no message is shown.
    await settleUntil(tester, () => checked.evaluate().isNotEmpty);
    expect(
      find.descendant(of: card, matching: find.text('Ship release')),
      findsOneWidget,
    );
    expect(find.text('Marked done, so it left this list.'), findsNothing);
    expect(await taskStatus(tester, task), 'done');

    await tester.tap(checked);
    await settleUntil(tester, () => unchecked.evaluate().isNotEmpty);
    expect(await taskStatus(tester, task), 'todo');
  });

  appTest('an open-work card drops a ticked task after the live update, and Undo brings it back', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final draft = await seedWorkTask(tester, 'Draft the migration note');
    final room = await seedWorkTask(tester, 'Book a room for the retro');
    await seedWorkTask(tester, 'Renew the code-signing cert', status: 'done');
    final block = await openDashboard(tester, query: '#work type:task -status:done');
    final card = inCard(block);
    Finder row(String id) =>
        find.descendant(of: card, matching: find.byKey(ValueKey('query-row-$id')));
    String countText() =>
        tester.widget<Text>(find.descendant(of: card, matching: find.byKey(const ValueKey('query-count')))).data!;

    // The done task is not an open task, so the card lists two rows.
    await settleUntil(tester, () => row(draft).evaluate().isNotEmpty && row(room).evaluate().isNotEmpty);
    expect(find.descendant(of: card, matching: find.text('Renew the code-signing cert')), findsNothing);
    expect(countText(), '2');

    await tester.tap(find.byKey(ValueKey('query-check-$draft')));
    // The tick hides its row at once. The message waits for the core's next
    // result, so it shows only once the core has dropped the row from the list.
    await settleUntil(tester, () => find.text('Marked done, so it left this list.').evaluate().isNotEmpty);
    expect(row(draft), findsNothing);
    expect(row(room), findsOneWidget);
    expect(countText(), '1');
    expect(await taskStatus(tester, draft), 'done');

    await tapAndSettle(tester, find.byKey(const ValueKey('query-toast-undo')));
    await settleUntil(tester, () => row(draft).evaluate().isNotEmpty && countText() == '2');
    expect(find.text('Marked done, so it left this list.'), findsNothing);
    expect(await taskStatus(tester, draft), 'todo');
  });

  appTest('the mockup page: its queries match their rows and not the host page', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    final page = await seedSearchCardPage(tester);
    // Four tagged task pages, three board rows in Doing, and the host page
    // kept out of its own "touched this week" card.
    expect(
      find.descendant(of: inCard(page.tasks), matching: find.text('4')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: inCard(page.board), matching: find.text('Doing')),
      findsNWidgets(3),
    );
    expect(
      find.descendant(of: inCard(page.week), matching: find.text('Week 42')),
      findsNothing,
    );
    // The week card holds more than its five preview rows.
    expect(
      find.descendant(of: inCard(page.week), matching: find.textContaining('Show ')),
      findsOneWidget,
    );
  });

  appTest('a query with no matches shows its empty line and a zero count', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Garden plan');
    final block = await openDashboard(tester, query: 'zzqxnomatch');
    await settleUntil(
      tester,
      () => find
          .descendant(
            of: inCard(block),
            matching: find.text('No results for zzqxnomatch.'),
          )
          .evaluate()
          .isNotEmpty,
    );
    expect(
      find.descendant(
        of: inCard(block),
        matching: find.text('No results for zzqxnomatch.'),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(of: inCard(block), matching: find.text('0')),
      findsOneWidget,
    );
  });

  appTest('more than five results show "Show N more" and expand in place', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    for (var i = 1; i <= 7; i++) {
      await seedPage(tester, 'Budget note $i');
    }
    final block = await openDashboard(tester, query: 'budget note');
    final card = inCard(block);
    await settleUntil(
      tester,
      () => find.descendant(of: card, matching: find.text('Show 2 more')).evaluate().isNotEmpty,
    );
    expect(
      find.descendant(of: card, matching: find.textContaining('Budget note')),
      findsNWidgets(5),
    );

    await tester.tap(find.descendant(of: card, matching: find.text('Show 2 more')));
    await settle(tester);
    expect(
      find.descendant(of: card, matching: find.textContaining('Budget note')),
      findsNWidgets(7),
    );
    expect(
      find.descendant(of: card, matching: find.text('Show 2 more')),
      findsNothing,
    );
  });

  appTest('Enter in the edit mode applies the query to the block', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    // A title no other test uses: the vault is shared by the tests in this
    // file, so a common title such as "Garden plan" would match their pages.
    await seedPage(tester, 'Orchard planning');
    final page = await seedPage(tester, 'Dashboard');
    final block = await seedQueryBlock(tester, page, 'zzqxnomatch');
    await openPageNamed(tester, page, 'Dashboard');
    await settleUntil(tester, () => inCard(block).evaluate().isNotEmpty);

    await tester.tap(find.byKey(ValueKey('query-text-$block')));
    await settle(tester);
    final field = find.byKey(ValueKey('query-edit-$block'));
    expect(field, findsOneWidget);
    await tester.enterText(field, 'orchard');
    tester.testTextInput.receiveAction(TextInputAction.done);
    await settleUntil(
      tester,
      () => find.descendant(of: inCard(block), matching: find.text('Orchard planning')).evaluate().isNotEmpty,
    );

    expect(find.byKey(ValueKey('query-edit-$block')), findsNothing);
    expect(await blockText(tester, page, block), 'orchard');
    expect(
      find.descendant(of: inCard(block), matching: find.text('Orchard planning')),
      findsOneWidget,
    );
  });

  appTest('Esc in the edit mode cancels and keeps the query', (tester) async {
    await pumpApp(tester, mobile: false);
    final page = await seedPage(tester, 'Dashboard');
    final block = await seedQueryBlock(tester, page, 'zzqxnomatch');
    await openPageNamed(tester, page, 'Dashboard');
    await settleUntil(tester, () => inCard(block).evaluate().isNotEmpty);

    await tester.tap(find.byKey(ValueKey('query-text-$block')));
    await settle(tester);
    final field = find.byKey(ValueKey('query-edit-$block'));
    await tester.enterText(field, 'changed my mind');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(tester);

    expect(find.byKey(ValueKey('query-edit-$block')), findsNothing);
    expect(await blockText(tester, page, block), 'zzqxnomatch');
  });

  appTest('a focused header opens the edit mode on Enter', (tester) async {
    await pumpApp(tester, mobile: false);
    final page = await seedPage(tester, 'Dashboard');
    final block = await seedQueryBlock(tester, page, 'zzqxnomatch');
    await openPageNamed(tester, page, 'Dashboard');
    await settleUntil(tester, () => inCard(block).evaluate().isNotEmpty);

    final header = tester.element(find.byKey(ValueKey('query-text-$block')));
    Focus.of(header).requestFocus();
    await settle(tester);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await settle(tester);
    expect(find.byKey(ValueKey('query-edit-$block')), findsOneWidget);
  });

  appTest('a card releases its watch when it goes away', (tester) async {
    final recorder = RecordingBridge();
    final previous = queryBridge;
    queryBridge = recorder;
    addTearDown(() => queryBridge = previous);
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Quarterly review');
    final dashboard = await seedPage(tester, 'Dashboard');
    final block = await seedQueryBlock(tester, dashboard, 'quarterly');
    await openPageNamed(tester, dashboard, 'Dashboard');
    await settleUntil(
      tester,
      () => find.descendant(of: inCard(block), matching: find.text('Quarterly review')).evaluate().isNotEmpty,
    );
    expect(recorder.live, hasLength(1));

    final context = tester.element(inCard(block));
    Navigator.of(context).maybePop();
    await settle(tester, min: const Duration(seconds: 1));
    await settleUntil(tester, () => inCard(block).evaluate().isEmpty);
    expect(recorder.live, isEmpty);
  });

  appTest('changing the query releases the old watch', (tester) async {
    final recorder = RecordingBridge();
    final previous = queryBridge;
    queryBridge = recorder;
    addTearDown(() => queryBridge = previous);
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Quarterly review');
    await seedPage(tester, 'Garden plan');
    final page = await seedPage(tester, 'Dashboard');
    final block = await seedQueryBlock(tester, page, 'quarterly');
    await openPageNamed(tester, page, 'Dashboard');
    await settleUntil(
      tester,
      () => find.descendant(of: inCard(block), matching: find.text('Quarterly review')).evaluate().isNotEmpty,
    );
    expect(recorder.live, hasLength(1));

    await tester.tap(find.byKey(ValueKey('query-text-$block')));
    await settle(tester);
    await tester.enterText(find.byKey(ValueKey('query-edit-$block')), 'garden');
    tester.testTextInput.receiveAction(TextInputAction.done);
    await settleUntil(
      tester,
      () => find.descendant(of: inCard(block), matching: find.text('Garden plan')).evaluate().isNotEmpty,
    );
    expect(recorder.live, hasLength(1));
    expect(
      find.descendant(of: inCard(block), matching: find.text('Quarterly review')),
      findsNothing,
    );
  });
}
