// Unit tests for the optimistic-tick rule (settlePendingTicks): a tick hides
// its row until its write finishes, then the next live result decides.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/search_card/query_hits.dart';
import 'package:notes_app/addons/search_card/query_ticks.dart';
import 'package:notes_app/src/rust/api/types.dart';

Hit taskHit(String id) => Hit(
  id: id,
  title: id,
  snippet: '',
  fuzzy: false,
  source: 'vault',
  pageType: 'task',
);

void main() {
  test('a tick whose write is still running stays pending', () {
    final hit = taskHit('a');
    final pending = {keyOfHit(hit): PendingTick(hit)};
    final left = settlePendingTicks(pending, [hit]);
    expect(left, isEmpty);
    expect(pending, hasLength(1));
  });

  test('a written tick whose row is still in the result comes back, with no message', () {
    final hit = taskHit('a');
    final pending = {keyOfHit(hit): PendingTick(hit)..writing = false};
    final left = settlePendingTicks(pending, [hit]);
    expect(left, isEmpty);
    expect(pending, isEmpty);
  });

  test('a written tick whose row the result dropped has left the list', () {
    final hit = taskHit('a');
    final pending = {keyOfHit(hit): PendingTick(hit)..writing = false};
    final left = settlePendingTicks(pending, [taskHit('b')]);
    expect(left.map((h) => h.id), ['a']);
    expect(pending, isEmpty);
  });

  test('only the finished ticks are settled; the others stay hidden', () {
    final done = taskHit('done');
    final running = taskHit('running');
    final pending = {
      keyOfHit(done): PendingTick(done)..writing = false,
      keyOfHit(running): PendingTick(running),
    };
    final left = settlePendingTicks(pending, const []);
    expect(left.map((h) => h.id), ['done']);
    expect(pending.keys, [keyOfHit(running)]);
  });

  test('a row from another source is a different row', () {
    final hit = taskHit('a');
    final other = Hit(
      id: 'a',
      title: 'a',
      snippet: '',
      fuzzy: false,
      source: 'agent',
      pageType: 'chat',
    );
    final pending = {keyOfHit(hit): PendingTick(hit)..writing = false};
    final left = settlePendingTicks(pending, [other]);
    expect(left.map((h) => h.id), ['a']);
  });
}
