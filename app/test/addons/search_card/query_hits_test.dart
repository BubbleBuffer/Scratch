// Unit tests for the live-query diff model. No bridge: updates are built by
// hand in the shape the core sends.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/search_card/query_hits.dart';
import 'package:notes_app/src/rust/api/types.dart';
import 'package:notes_app/src/rust/api/vault.dart';

Hit hit(String id, {String title = '', String source = 'vault'}) => Hit(
  id: id,
  title: title.isEmpty ? id : title,
  snippet: '',
  fuzzy: false,
  source: source,
  pageType: 'document',
);

HitKey key(String id, {String source = 'vault'}) =>
    HitKey(source: source, id: id);

LiveQueryUpdate update({
  bool initial = false,
  List<Hit> added = const [],
  List<HitKey> removed = const [],
  List<HitMove> moved = const [],
  List<Hit> changed = const [],
  int total = 0,
  int generation = 1,
}) => LiveQueryUpdate(
  handle: BigInt.one,
  generation: BigInt.from(generation),
  initial: initial,
  added: added,
  removed: removed,
  moved: moved,
  changed: changed,
  total: total,
);

List<String> ids(QueryHitList list) => [for (final h in list.hits) h.id];

void main() {
  test('the initial snapshot keeps its order, even when empty', () {
    final list = QueryHitList();
    list.apply(
      update(initial: true, added: [hit('a'), hit('b'), hit('c')], total: 3),
    );
    expect(ids(list), ['a', 'b', 'c']);
    expect(list.total, 3);

    final empty = QueryHitList();
    empty.apply(update(initial: true, total: 0));
    expect(empty.hits, isEmpty);
    expect(empty.total, 0);
  });

  test('a new initial snapshot replaces what was there', () {
    final list = QueryHitList();
    list.apply(update(initial: true, added: [hit('a'), hit('b')], total: 2));
    list.apply(update(initial: true, added: [hit('z')], total: 1));
    expect(ids(list), ['z']);
  });

  test('added hits are appended in order', () {
    final list = QueryHitList();
    list.apply(update(initial: true, added: [hit('a')], total: 1));
    list.apply(update(added: [hit('b'), hit('c')], total: 3));
    expect(ids(list), ['a', 'b', 'c']);
    expect(list.total, 3);
  });

  test('removed hits leave and the rest keep their order', () {
    final list = QueryHitList();
    list.apply(
      update(initial: true, added: [hit('a'), hit('b'), hit('c')], total: 3),
    );
    list.apply(update(removed: [key('b')], total: 2));
    expect(ids(list), ['a', 'c']);
    expect(list.total, 2);
  });

  test('a removal of an unknown key is ignored', () {
    final list = QueryHitList();
    list.apply(update(initial: true, added: [hit('a')], total: 1));
    list.apply(update(removed: [key('missing')], total: 1));
    expect(ids(list), ['a']);
  });

  test('a move places the hit at its final index', () {
    final list = QueryHitList();
    list.apply(
      update(initial: true, added: [hit('a'), hit('b'), hit('c')], total: 3),
    );
    // c moves to the front; a and b keep their relative order.
    list.apply(
      update(
        moved: [HitMove(key: key('c'), newIndex: 0)],
        total: 3,
      ),
    );
    expect(ids(list), ['c', 'a', 'b']);
  });

  test('several moves are placed simultaneously', () {
    final list = QueryHitList();
    list.apply(
      update(
        initial: true,
        added: [hit('a'), hit('b'), hit('c'), hit('d')],
        total: 4,
      ),
    );
    // Swap the outer pair; the middle two stay in order.
    list.apply(
      update(
        moved: [
          HitMove(key: key('a'), newIndex: 3),
          HitMove(key: key('d'), newIndex: 0),
        ],
        total: 4,
      ),
    );
    expect(ids(list), ['d', 'b', 'c', 'a']);
  });

  test('a new hit with a move entry lands at its index, not the end', () {
    final list = QueryHitList();
    list.apply(update(initial: true, added: [hit('a'), hit('c')], total: 2));
    list.apply(
      update(
        added: [hit('b')],
        moved: [HitMove(key: key('b'), newIndex: 1)],
        total: 3,
      ),
    );
    expect(ids(list), ['a', 'b', 'c']);
    expect(list.total, 3);
  });

  test('changed hits replace their data and keep their position', () {
    final list = QueryHitList();
    list.apply(
      update(
        initial: true,
        added: [hit('a', title: 'Old A'), hit('b', title: 'B')],
        total: 2,
      ),
    );
    list.apply(update(changed: [hit('a', title: 'New A')], total: 2));
    expect(ids(list), ['a', 'b']);
    expect(list.hits.first.title, 'New A');
  });

  test('a removal, an addition and a move in one update apply in order', () {
    final list = QueryHitList();
    list.apply(
      update(initial: true, added: [hit('a'), hit('b'), hit('c')], total: 3),
    );
    list.apply(
      update(
        removed: [key('a')],
        added: [hit('d')],
        moved: [HitMove(key: key('d'), newIndex: 0)],
        changed: [hit('c', title: 'C2')],
        total: 3,
      ),
    );
    expect(ids(list), ['d', 'b', 'c']);
    expect(list.hits.last.title, 'C2');
  });

  test('keys include the source, so equal ids from two sources differ', () {
    final list = QueryHitList();
    list.apply(
      update(
        initial: true,
        added: [hit('1'), hit('1', source: 'chat')],
        total: 2,
      ),
    );
    list.apply(update(removed: [key('1', source: 'chat')], total: 1));
    expect(list.hits.map((h) => h.source), ['vault']);
  });

  test('an update without changes only moves the total', () {
    final list = QueryHitList();
    list.apply(update(initial: true, added: [hit('a')], total: 1));
    list.apply(update(total: 1, generation: 2));
    expect(ids(list), ['a']);
    expect(list.total, 1);
  });

  test('the hit list cannot be changed from outside', () {
    final list = QueryHitList();
    list.apply(update(initial: true, added: [hit('a')], total: 1));
    expect(() => list.hits.add(hit('x')), throwsUnsupportedError);
  });
}
