// Compacted history without the app: the mapping of the bridge's history onto
// the kit's cards, and the entries the transcript is made of. The widgets are
// in agent_history_test.dart.

import 'package:agent_kit/agent_kit.dart' show HistoryController;
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/agent_history.dart';
import 'package:notes_app/addons/agent/agent_transcript.dart';
import 'package:notes_app/src/rust/api/agent.dart';

import 'agent_history_fixture.dart';

void main() {
  test(
    'every covered message gets the key of its episode, card or stretch',
    () {
      final keys = historySegments(historyOf());
      expect(keys['u0'], 'lg:lg1');
      expect(keys[historyMessageId(6)], 'ep:ep1');
      expect(keys[historyMessageId(23)], 'ep:ep2');
      expect(keys[historyMessageId(30)], 'm:3');
      expect(keys[historyMessageId(32)], 'ep:ep4');
      expect(keys[historyMessageId(43)], 'ep:ep5');
      expect(keys[historyMessageId(44)], 'm:6');
      expect(keys[historyMessageId(59)], 'm:6');
      expect(keys.length, historyPairs * 2);
    },
  );

  test('a chat with no card, or one the store cannot summarize, has none', () {
    expect(hasHistoryCards(historyOf()), isTrue);
    expect(hasHistoryCards(historyOf(compactable: false)), isFalse);
    expect(hasHistoryCards(null), isFalse);
    expect(
      hasHistoryCards(
        CompactedHistory(
          compactable: true,
          boundary: 0,
          messageCount: 2,
          messageIds: const ['u0', 'a0'],
          segments: const [
            HistorySegment(
              kind: HistorySegmentKind.messages,
              range: MessageRange(start: 0, end: 2),
            ),
          ],
        ),
      ),
      isFalse,
    );
  });

  test('a split chapter keeps its id per part and the counts on the last', () {
    final chapters = [
      for (final s in historyOf().segments)
        if (s.chapter != null) s.chapter!,
    ];
    final parts = [for (final (i, c) in chapters.indexed) historyChapter(c, i)];
    expect(parts.map((c) => c.key), ['ch1', 'ch2#1', 'ch2#2', 'ch3']);
    expect(parts[1].messageCount, isNull);
    expect(parts[1].episodeCount, isNull);
    expect(parts[2].messageCount, 12);
    expect(parts[2].episodeCount, 2);
    expect(parts[2].title, parts[1].title);
    expect(parts[0].closed, isTrue);
    expect(parts[3].closed, isFalse);
    expect(parts[3].endedAt, isNull);
  });

  test(
    'decisions and sections map through; a chapter without them has none',
    () {
      final chapters = [
        for (final s in historyOf().segments)
          if (s.chapter != null) s.chapter!,
      ];
      final parts = [
        for (final (i, c) in chapters.indexed) historyChapter(c, i),
      ];
      expect(parts[0].decisions, ['Hold the last value on reconnect.']);
      expect(parts[0].sections.map((s) => s.name), [
        'Find the cause',
        'Hold the figure',
      ]);
      expect(parts[0].sections.last.firstEpisode, 2);
      // Split chapter: the last part carries them, the first none.
      expect(parts[1].decisions, isEmpty);
      expect(parts[1].sections, isEmpty);
      expect(parts[2].decisions, ['Stop after five tries.']);
      expect(parts[2].sections, isEmpty);
      expect(parts[3].decisions, isEmpty);
    },
  );

  test('an episode keeps its actions, markers and flagged points', () {
    final chapters = [
      for (final s in historyOf().segments)
        if (s.chapter != null) s.chapter!,
    ];
    final first = historyEpisode(chapters.first.episodes.first);
    expect(first.hasActions, isTrue);
    expect(first.actions, hasLength(8));
    expect(first.actions[1].marker, '+12 −3');
    expect(first.actions[2].point?.reason, 'socket closes early');
    expect(first.flaggedCount, 1);
    expect(first.errorCount, 1);
    final v2 = historyEpisode(chapters.first.episodes.last);
    expect(v2.hasActions, isFalse);
    expect(v2.actionCount, 0);
    expect(v2.gist, isNotNull);
  });

  test(
    'the entries run cards, the live stretch, cards, the hairline, live',
    () {
      final history = historyOf();
      final anchors = <String, String>{};
      final items = agentTurns(
        historyChat('s'),
        anchorOf: anchors,
        segmentOf: historySegments(history),
      );
      final built = compactedEntries(
        history: history,
        items: items,
        anchors: anchors,
        segmentOf: historySegments(history),
        controller: HistoryController(),
        itemView: (_) => const SizedBox(),
      );
      final ids = [for (final e in built.entries) e.id];
      expect(ids.take(9).toList(), [
        'history-legacy-lg1',
        'history-chapter-ch1',
        'history-chapter-ch2#1',
        'u15',
        'message-a15#m:3',
        'history-chapter-ch2#2',
        'history-chapter-ch3',
        historyBoundaryId,
        'u22',
      ]);
      // The live end is every message from the boundary, once.
      expect(ids.skip(8).where((i) => i.startsWith('u')).length, 8);
      expect(ids.where((i) => i == historyBoundaryId), hasLength(1));
      // Hidden messages are not entries of their own.
      expect(ids, isNot(contains('u3')));
      // A message is found at its card, or at its own row in the live end.
      expect(built.entryOf['u0'], 'history-legacy-lg1');
      expect(built.entryOf[historyMessageId(7)], 'history-chapter-ch1');
      expect(built.entryOf[historyMessageId(33)], 'history-chapter-ch2#2');
      expect(built.entryOf[historyMessageId(46)], 'u23');
    },
  );
}
