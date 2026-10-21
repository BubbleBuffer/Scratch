import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/sync/conflicts.dart';

Conflict _entry({
  Resolution? resolution,
  ResolutionBy by = ResolutionBy.user,
  int? undoneAt,
  bool localFirst = false,
  bool applied = true,
}) => Conflict(
  id: 'conflict',
  format: 2,
  page: 'page',
  block: 'block',
  timeMs: 0,
  scope: const ConflictScope(kind: ConflictScopeKind.block, block: 'block'),
  sides: [
    for (var i = 0; i < 2; i++)
      ConflictSide(
        device: 'device-$i',
        local: i == (localFirst ? 0 : 1),
        timeMs: 0,
        state: const ConflictSideState(
          kind: ConflictStateKind.present,
          content: [],
        ),
      ),
  ],
  status: ConflictStatus.resolved,
  applied: [
    if (applied)
      AppliedResolution(
        timeMs: 1,
        resolution: resolution,
        undoneAt: undoneAt,
        by: by,
        before: Uint8List(0),
        fallback: false,
        restoredAncestors: const [],
      ),
  ],
  undoable: false,
);

void main() {
  test('Keep reads Kept', () {
    expect(
      conflictLogWhat(
        _entry(
          resolution: const Resolution(kind: ResolutionKind.keep, side: 0),
        ),
      ),
      'Kept',
    );
  });

  test('UseSide names the chosen side by local identity, in either order', () {
    for (final localFirst in [false, true]) {
      for (var i = 0; i < 2; i++) {
        final c = _entry(
          resolution: Resolution(kind: ResolutionKind.useSide, side: i),
          localFirst: localFirst,
        );
        expect(
          conflictLogWhat(c),
          c.sides[i].local
              ? 'Used the local version'
              : 'Used the cloud version',
        );
      }
    }
  });

  test('legacy applications without a resolution read Resolved', () {
    expect(conflictLogWhat(_entry()), 'Resolved');
    expect(conflictLogWhat(_entry(applied: false)), 'Resolved');
  });

  test('an unavailable side reads Resolved', () {
    for (final i in [-1, 2]) {
      expect(
        conflictLogWhat(
          _entry(
            resolution: Resolution(kind: ResolutionKind.useSide, side: i),
          ),
        ),
        'Resolved',
      );
    }
  });

  test('automatic applications retain Settled on its own for every choice', () {
    for (final choice in [
      null,
      const Resolution(kind: ResolutionKind.keep, side: 0),
      const Resolution(kind: ResolutionKind.useSide, side: 1),
    ]) {
      expect(
        conflictLogWhat(_entry(resolution: choice, by: ResolutionBy.auto)),
        'Settled on its own',
      );
    }
  });

  test('Undone takes precedence over the choice and author', () {
    for (final by in ResolutionBy.values) {
      for (final choice in [
        null,
        const Resolution(kind: ResolutionKind.keep, side: 0),
        const Resolution(kind: ResolutionKind.useSide, side: 1),
      ]) {
        expect(
          conflictLogWhat(_entry(resolution: choice, by: by, undoneAt: 2)),
          'Undone',
        );
      }
    }
  });
}
