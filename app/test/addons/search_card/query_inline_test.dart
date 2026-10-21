// The inline query chip (Form B). A paragraph's query mark shows the live count
// of its query, a tap opens the results panel, and the panel converts the
// paragraph to or from a query block. Each conversion is one undo step.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/search_card/query_block.dart';
import 'package:notes_app/addons/search_card/query_card.dart';
import 'package:notes_app/core/models.dart';

import 'search_card_fixture.dart';

const _query = 'quarterly';

Future<FlatBlock> blockOf(WidgetTester tester, String page, String id) async =>
    (await blocksOf(tester, page)).firstWhere((b) => b.id == id);

Future<void> _undo(WidgetTester tester) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await settle(tester);
}

void main() {
  setUpAll(initCore);

  // ---- conversions ----

  appTest('"Show inline" turns a query block into a chip, and one undo restores it', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Quarterly review');
    final page = await seedPage(tester, 'Dashboard');
    final block = await seedQueryBlock(tester, page, _query);
    await openPageNamed(tester, page, 'Dashboard');
    await settleUntil(
      tester,
      () => find
          .descendant(of: inCard(block), matching: find.text('Quarterly review'))
          .evaluate()
          .isNotEmpty,
    );

    await tapAndSettle(tester, find.byKey(ValueKey('query-overflow-$block')));
    await tapAndSettle(tester, find.text('Show inline'));

    final chip = await blockOf(tester, page, block);
    expect(chip.kind, Kinds.paragraph);
    expect(chip.text, isNotEmpty);
    expect(chip.marks, hasLength(1));
    final mark = chip.marks.single;
    expect(mark.kind, BMarkKind.query);
    expect(mark.url, _query);
    expect(mark.start, 0);
    expect(mark.end, chip.text.length);
    await settleUntil(
      tester,
      () => chipLabelOf(tester, chip.text, _query) == chip.text,
    );
    expect(inCard(block), findsNothing);

    await focusAt(tester, editableWith(chip.text), 0);
    await _undo(tester);

    final restored = await blockOf(tester, page, block);
    expect(restored.kind, searchQueryKind);
    expect(restored.text, _query);
    expect(restored.marks, isEmpty);
    await settleUntil(tester, () => inCard(block).evaluate().isNotEmpty);
  });

  appTest('"Show as a block" on a chip alone turns the paragraph into the query block', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Quarterly review');
    final page = await seedPage(tester, 'Week');
    final para = await seedChip(
      tester,
      page,
      'Open work',
      query: _query,
      start: 0,
      end: 9,
    );
    await openChipPage(tester, page, 'Week', text: 'Open work', query: _query);

    await tapChip(tester, 'Open work');
    expect(find.byKey(const ValueKey('query-panel')), findsOneWidget);
    await tapAndSettle(tester, find.byKey(const ValueKey('query-panel-as-block')));

    final block = await blockOf(tester, page, para);
    expect(block.kind, searchQueryKind);
    expect(block.text, _query);
    expect(block.marks, isEmpty);
    await settleUntil(tester, () => inCard(para).evaluate().isNotEmpty);

    // Undo needs a focused node in the page; the card's header is one.
    Focus.of(tester.element(find.byKey(ValueKey('query-text-$para')))).requestFocus();
    await settle(tester);
    await _undo(tester);

    final restored = await blockOf(tester, page, para);
    expect(restored.kind, Kinds.paragraph);
    expect(restored.text, 'Open work');
    expect(restored.marks.single.url, _query);
    expect(restored.marks.single.end, 9);
    expect(inCard(para), findsNothing);
  });

  appTest('"Show as a block" after other text inserts the block after the paragraph', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Quarterly review');
    final page = await seedPage(tester, 'Week');
    final para = await seedChip(
      tester,
      page,
      'quarterly and more',
      query: _query,
      start: 0,
      end: 9,
    );
    await openChipPage(
      tester,
      page,
      'Week',
      text: 'quarterly and more',
      query: _query,
    );

    await tapChip(tester, 'quarterly and more');
    await tapAndSettle(tester, find.byKey(const ValueKey('query-panel-as-block')));

    final blocks = await blocksOf(tester, page);
    final index = blocks.indexWhere((b) => b.id == para);
    final text = blocks[index];
    final added = blocks[index + 1];
    expect(text.text, 'and more');
    expect(text.marks, isEmpty);
    expect(added.kind, searchQueryKind);
    expect(added.text, _query);
    expect(blocks, hasLength(2));

    // One undo removes the new block and puts the chip's text back.
    await focusAt(tester, editableWith('and more'), 0);
    await _undo(tester);

    final restored = await blocksOf(tester, page);
    expect(restored.any((b) => b.kind == searchQueryKind), isFalse);
    final back = restored.firstWhere((b) => b.id == para);
    expect(back.text, 'quarterly and more');
    expect(back.marks.single.url, _query);
    expect(back.marks.single.end, 9);
  });

  appTest('"Show as a block" removes one space with the chip, and no other whitespace', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Quarterly review');
    final page = await seedPage(tester, 'Week');
    final para = await seedChip(
      tester,
      page,
      'quarterly  and more',
      query: _query,
      start: 0,
      end: 9,
    );
    await openChipPage(
      tester,
      page,
      'Week',
      text: 'quarterly  and more',
      query: _query,
    );

    await tapChip(tester, 'quarterly  and more');
    await tapAndSettle(tester, find.byKey(const ValueKey('query-panel-as-block')));

    // Two spaces follow the chip: one goes with it, the other stays.
    final blocks = await blocksOf(tester, page);
    final text = blocks.firstWhere((b) => b.id == para);
    expect(text.text, ' and more');
    expect(blocks.where((b) => b.kind == searchQueryKind), hasLength(1));
  });

  // ---- the chip ----

  appTest('the chip shows the live count and follows new matches', (tester) async {
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Quarterly review');
    final page = await seedPage(tester, 'Week');
    await seedChip(tester, page, 'Open work', query: _query, start: 0, end: 9);
    await openChipPage(tester, page, 'Week', text: 'Open work', query: _query);
    final before = chipLabelOf(tester, 'Open work', _query);

    await seedPage(tester, 'Quarterly budget');
    await settleUntil(
      tester,
      () => chipLabelOf(tester, 'Open work', _query) != before,
    );
    expect(chipLabelOf(tester, 'Open work', _query), isNot(before));
    expect(tester.takeException(), isNull);
  });

  appTest('a tap on the chip opens the results panel as a desktop popover', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Quarterly review');
    final page = await seedPage(tester, 'Week');
    await seedChip(tester, page, 'Open work', query: _query, start: 0, end: 9);
    await openChipPage(tester, page, 'Week', text: 'Open work', query: _query);

    await tapChip(tester, 'Open work');
    expect(find.byKey(const ValueKey('query-panel')), findsOneWidget);
    expect(
      find.byKey(const ValueKey('query-panel-open-search')),
      findsOneWidget,
    );
    expect(tester.takeException(), isNull);
  });

  appTest('a tap on the chip opens the results panel as a phone sheet', (
    tester,
  ) async {
    await pumpApp(tester, mobile: true);
    await seedPage(tester, 'Quarterly review');
    final page = await seedPage(tester, 'Week');
    await seedChip(tester, page, 'Open work', query: _query, start: 0, end: 9);
    await openChipPage(tester, page, 'Week', text: 'Open work', query: _query);

    await tapChip(tester, 'Open work');
    expect(find.byKey(const ValueKey('query-panel')), findsNothing);
    expect(
      find.byKey(const ValueKey('query-panel-open-search')),
      findsOneWidget,
    );
    expect(tester.takeException(), isNull);
  });

  appTest('leaving the page releases the chip watch', (tester) async {
    final recorder = RecordingBridge();
    final previous = queryBridge;
    queryBridge = recorder;
    addTearDown(() => queryBridge = previous);
    await pumpApp(tester, mobile: false);
    await seedPage(tester, 'Quarterly review');
    final page = await seedPage(tester, 'Week');
    await seedChip(tester, page, 'Open work', query: _query, start: 0, end: 9);
    await openChipPage(tester, page, 'Week', text: 'Open work', query: _query);
    expect(recorder.live, hasLength(1));

    final context = tester.element(editableWith('Open work'));
    Navigator.of(context).maybePop();
    await settle(tester, min: const Duration(seconds: 1));
    await settleUntil(tester, () => editableWith('Open work').evaluate().isEmpty);
    expect(recorder.live, isEmpty);
  });
}
