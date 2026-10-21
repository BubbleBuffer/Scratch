// Which side a block changed on both devices shows: conflictShownSide. A plain
// unit test; no app and no bridge. The core records the sides; this helper
// states the rule the note and its label follow: the later edit wins, equal
// times go to the greater device id.

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/sync/conflicts.dart';

ConflictSide _side(String device, int timeMs, {bool local = false}) => ConflictSide(
      device: device,
      local: local,
      timeMs: timeMs,
      state: const ConflictSideState(kind: ConflictStateKind.present, content: []),
    );

Conflict _conflict(List<ConflictSide> sides) => Conflict(
      id: 'c1',
      format: 1,
      page: 'p1',
      block: 'b1',
      timeMs: 0,
      scope: const ConflictScope(kind: ConflictScopeKind.block, block: 'b1'),
      sides: sides,
      status: ConflictStatus.pending,
      applied: const [],
      undoable: false,
    );

void main() {
  test('the side with the later edit is shown, whichever index it has', () {
    expect(conflictShownSide(_conflict([_side('a', 200), _side('b', 100)])), 0);
    expect(conflictShownSide(_conflict([_side('a', 100), _side('b', 200)])), 1);
  });

  test('equal edit times: the greater device id is shown, whichever index it has', () {
    expect(conflictShownSide(_conflict([_side('alpha', 50), _side('beta', 50)])), 1);
    expect(conflictShownSide(_conflict([_side('beta', 50), _side('alpha', 50)])), 0);
  });

  test('one side: that side is shown', () {
    expect(conflictShownSide(_conflict([_side('alpha', 1, local: true)])), 0);
  });

  test('no sides: nothing is shown', () {
    expect(conflictShownSide(_conflict(const [])), isNull);
  });
}
