// The optimistic ticks of a search card's task rows. A tick hides its row at
// once; when its status write has finished, the next live result decides:
// a row the result still holds comes back checked, a row it no longer holds
// has left the list. Pure Dart, so the rule is unit-tested without the core.

import '../../src/rust/api/types.dart';
import '../../src/rust/api/vault.dart';
import 'query_hits.dart';

/// One tick waiting for its status write and then the core's next result.
class PendingTick {
  PendingTick(this.hit);

  final Hit hit;

  /// True until the status write has finished.
  bool writing = true;
}

/// Removes the ticks whose write has finished from [pending], and returns
/// the hits among them that [hits] no longer holds (the rows that left).
/// Ticks whose write is still running stay pending and are not returned.
List<Hit> settlePendingTicks(
  Map<HitKey, PendingTick> pending,
  List<Hit> hits,
) {
  final present = {for (final hit in hits) keyOfHit(hit)};
  final left = <Hit>[];
  for (final entry in pending.entries.toList()) {
    if (entry.value.writing) continue;
    pending.remove(entry.key);
    if (!present.contains(entry.key)) left.add(entry.value.hit);
  }
  return left;
}
