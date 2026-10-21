// The chat outline's layout: folded chapters, open chapters with section
// dividers, grouped turns that hide individual message anchors, and the
// targets that jumps resolve to. Pure Dart, no widgets.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';

// Messages in order: u1 a1 t1 u2 a2 u3 a3 u4. Turn r1 groups a1 and its tool
// result t1; r2 holds a2; r3 holds a3. Users are their own items.
const _messages = ['u1', 'a1', 't1', 'u2', 'a2', 'u3', 'a3', 'u4'];
const _anchorOf = {
  'u1': 'u1',
  'a1': 'r1',
  't1': 'r1',
  'u2': 'u2',
  'a2': 'r2',
  'u3': 'u3',
  'a3': 'r3',
  'u4': 'u4',
};

const _items = <ChatItem>[
  UserText('u1', 'one'),
  AgentTurn(
    'r1',
    texts: [],
    cells: [],
    activity: [],
    edits: [],
  ),
  UserText('u2', 'two'),
  AgentTurn('r2', texts: [], cells: [], activity: [], edits: []),
  UserText('u3', 'three'),
  AgentTurn('r3', texts: [], cells: [], activity: [], edits: []),
  UserText('u4', 'four'),
];

// Chapter A (closed): u1..t1, sections S1 u1..a1 and S2 t1..t1.
// Chapter B (ongoing): u2..a3, sections S3 u2..a2 and S4 u3..a3.
const _outline = OutlineView([
  ChapterView(
    id: 'A',
    title: 'Chapter A',
    first: 'u1',
    last: 't1',
    decisions: ['Keep A'],
    ongoing: false,
    sections: [
      SectionView(id: 'S1', title: 'Section 1', first: 'u1', last: 'a1'),
      SectionView(id: 'S2', title: 'Section 2', first: 't1', last: 't1'),
    ],
  ),
  ChapterView(
    id: 'B',
    title: 'Chapter B',
    first: 'u2',
    last: 'a3',
    ongoing: true,
    sections: [
      SectionView(id: 'S3', title: 'Section 3', first: 'u2', last: 'a2'),
      SectionView(id: 'S4', title: 'Section 4', first: 'u3', last: 'a3'),
    ],
  ),
]);

OutlineLayout _layout({
  OutlineView outline = _outline,
  List<ChatItem> items = _items,
  bool Function(ChapterView) folded = _defaultFold,
}) => OutlineLayout.build(
  outline: outline,
  items: items,
  messageIds: _messages,
  anchorOf: _anchorOf,
  folded: folded,
);

bool _defaultFold(ChapterView c) => c.foldedByDefault;

List<String> _keys(OutlineLayout l) => [for (final r in l.rows) r.key];

void main() {
  test('no outline: rows are the items, in order, unchanged', () {
    final l = _layout(outline: OutlineView.empty);
    expect(_keys(l), [for (final i in _items) 'item:${i.id}']);
    expect(l.rows.every((r) => r.chapter == -1 && r.section == -1), isTrue);
  });

  test('closed chapter folds to a card; ongoing chapter opens with dividers', () {
    final l = _layout();
    expect(_keys(l), [
      'chap:A',
      'chap:B',
      'sec:S3',
      'item:u2',
      'item:r2',
      'sec:S4',
      'item:u3',
      'item:r3',
      'item:u4',
    ]);
    final card = l.rows.first as ChapterRow;
    expect(card.folded, isTrue);
    expect((l.rows[1] as ChapterRow).folded, isFalse);
  });

  test('messages outside every chapter are kept before and after', () {
    final l = _layout();
    expect(l.rows.last.key, 'item:u4');
    expect(l.rows.last.chapter, -1);
  });

  test('unfolding a closed chapter shows its messages under the heading', () {
    final l = _layout(folded: (c) => false);
    expect(_keys(l).take(4).toList(), [
      'chap:A',
      'sec:S1',
      'item:u1',
      // r1 starts at a1, inside S1: its divider is S1 and S2 has no item of
      // its own, so S2 shows no divider and its jump lands on r1.
      'item:r1',
    ]);
  });

  test('a grouped turn hides its tool messages: their anchor is the turn', () {
    final l = _layout();
    final t = l.target('t1', 't1');
    expect(t?.itemId, 'r1');
    expect(t?.chapter, 0);
    // The turn is hidden inside the folded chapter A.
    expect(l.rowIndex('item:r1'), isNull);
  });

  test('a jump into a folded chapter reports the chapter to unfold', () {
    final l = _layout();
    final t = l.target('a1', 'a1');
    expect(t?.itemId, 'r1');
    expect(t?.chapter, 0);
    final unfolded = _layout(folded: (c) => c.id != 'A' && c.foldedByDefault);
    expect(unfolded.rowIndex('item:r1'), isNotNull);
  });

  test('a turn that runs past a folded chapter stays visible', () {
    // Chapter A covers u1 and a1 only, so turn r1 (a1, t1) starts inside A
    // and ends after it: it is shown under the folded card.
    const outline = OutlineView([
      ChapterView(id: 'A', title: 'A', first: 'u1', last: 'a1'),
    ]);
    final l = _layout(outline: outline, folded: (c) => true);
    expect(_keys(l).take(3).toList(), ['chap:A', 'item:r1', 'item:u2']);
    expect(l.rowIndex('item:u1'), isNull);
  });

  test('a span is flashed by the rows of the items holding its messages', () {
    final l = _layout();
    expect(l.spanKeys('a1', 'a2'), {'item:r1', 'item:u2', 'item:r2'});
  });

  test('a missing first id falls back to the last id, and vice versa', () {
    final l = _layout();
    expect(l.target('nope', 'a2')?.itemId, 'r2');
    expect(l.target('a2', 'nope')?.itemId, 'r2');
    expect(l.target('nope', 'nope'), isNull);
  });

  test('a span with no shown message has no target', () {
    final l = _layout(items: const [UserText('u4', 'four')]);
    expect(l.target('u1', 'a1'), isNull);
    expect(l.target('u4', 'u4')?.itemId, 'u4');
  });

  test('the chapter an item sits in is known even when it is hidden', () {
    final l = _layout();
    expect(l.chapterOfItem('r1'), 0);
    expect(l.chapterOfItem('r3'), 1);
    expect(l.chapterOfItem('u4'), -1);
  });

  test('rows carry the chapter and section they belong to', () {
    final l = _layout();
    final r2 = l.rows.firstWhere((r) => r.key == 'item:r2');
    expect(r2.chapter, 1);
    expect(r2.section, 0);
    final s4 = l.rows.firstWhere((r) => r.key == 'sec:S4');
    expect(s4.chapter, 1);
    expect(s4.section, 1);
  });
}
