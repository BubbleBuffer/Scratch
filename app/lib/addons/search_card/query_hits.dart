// The ordered hits of one live query, kept by applying the core's updates.
// Pure Dart: no widgets, no bridge calls, so the diff rules are unit-tested.

import '../../src/rust/api/types.dart';
import '../../src/rust/api/vault.dart';

HitKey keyOfHit(Hit hit) => HitKey(source: hit.source, id: hit.id);

class QueryHitList {
  final _hits = <Hit>[];
  int total = 0;

  List<Hit> get hits => List.unmodifiable(_hits);

  /// Applies one update in the order the core documents: an initial update
  /// replaces everything; otherwise removals go first, additions are
  /// appended, moves are placed at their final indices all at once, and the
  /// hit data in `changed` is replaced last.
  void apply(LiveQueryUpdate update) {
    if (update.initial) _hits.clear();
    final removed = update.removed.toSet();
    _hits.removeWhere((h) => removed.contains(keyOfHit(h)));
    for (final hit in update.added) {
      _hits.removeWhere((h) => keyOfHit(h) == keyOfHit(hit));
      _hits.add(hit);
    }
    _placeMoves(update.moved);
    for (final hit in update.changed) {
      final i = _hits.indexWhere((h) => keyOfHit(h) == keyOfHit(hit));
      if (i >= 0) _hits[i] = hit;
    }
    total = update.total;
  }

  /// Lifts every moved hit out, then drops them back in at their target
  /// indices. The unmoved hits keep their order and fill the other slots.
  void _placeMoves(List<HitMove> moves) {
    if (moves.isEmpty) return;
    final targets = {for (final m in moves) m.key: m.newIndex};
    final moved = <HitKey, Hit>{};
    final rest = <Hit>[];
    for (final h in _hits) {
      final key = keyOfHit(h);
      if (targets.containsKey(key)) {
        moved[key] = h;
      } else {
        rest.add(h);
      }
    }
    final size = rest.length + moved.length;
    final placed = List<Hit?>.filled(size, null);
    final overflow = <Hit>[];
    for (final entry in moved.entries) {
      final index = targets[entry.key]!;
      if (index < size && placed[index] == null) {
        placed[index] = entry.value;
      } else {
        overflow.add(entry.value);
      }
    }
    var next = 0;
    final pending = [...rest, ...overflow];
    for (var i = 0; i < size; i++) {
      if (placed[i] == null) placed[i] = pending[next++];
    }
    _hits
      ..clear()
      ..addAll(placed.cast<Hit>());
  }
}
