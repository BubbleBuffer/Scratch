// Agent search rows: labels, anchors, and how the index follows the chats.
// Fakes stand in for the agent and the vault index, so no bridge is needed.
import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/src/rust/api/agent.dart';

AgentSession session(
  String id,
  String title, {
  String updated = '2026-10-10T10:00:00Z',
}) => AgentSession(
  id: id,
  title: title,
  createdAt: '2026-10-10T09:00:00Z',
  updatedAt: updated,
);

OutlineSection section(
  String first,
  String title, {
  String summary = 'Section summary.',
}) => OutlineSection(
  id: 'section-$first',
  title: title,
  summary: summary,
  decisions: const [],
  firstMessageId: first,
  lastMessageId: first,
);

OutlineChapter chapter(
  String first,
  String title, {
  String summary = 'Chapter summary.',
  List<OutlineSection> sections = const [],
}) => OutlineChapter(
  id: 'chapter-$first',
  title: title,
  summary: summary,
  decisions: const [],
  ongoing: false,
  firstMessageId: first,
  lastMessageId: '$first-end',
  startedAt: '2026-10-10T09:00:00Z',
  endedAt: '2026-10-10T09:30:00Z',
  sections: sections,
);

ChatOutline outlineOf(List<OutlineChapter> chapters) =>
    ChatOutline(chapters: chapters);

class FakeSink implements AgentSearchSink {
  final rows = <String, AgentSearchRow>{};
  final deleted = <String>[];
  int clears = 0;

  /// Every call to the sink, so a test can see that nothing ran.
  int calls = 0;
  bool failDeletes = false;

  @override
  Future<void> upsert(AgentSearchRow row) async {
    calls++;
    rows[row.id] = row;
  }

  @override
  Future<void> delete(String id) async {
    calls++;
    if (failDeletes) throw StateError('delete failed $id');
    deleted.add(id);
    rows.remove(id);
  }

  @override
  Future<void> clear() async {
    calls++;
    clears++;
    rows.clear();
  }

  Iterable<String> get ids => rows.keys;
}

class FakeReader implements AgentSearchReader {
  List<AgentSession> listed = [];
  final revisions = <String, BigInt>{};
  final outlines = <String, ChatOutline>{};
  final failing = <String>{};
  final outlineReads = <String>[];
  bool offline = false;
  Completer<void>? gate;
  String? gated;

  @override
  Future<List<AgentSession>> sessions() async {
    if (offline) throw StateError('agent offline');
    return List.of(listed);
  }

  @override
  Future<BigInt> revision(String sessionId) async {
    if (failing.contains(sessionId)) throw StateError('no outline $sessionId');
    return revisions[sessionId] ?? BigInt.zero;
  }

  @override
  Future<ChatOutline> outline(String sessionId) async {
    if (gated == sessionId && gate != null) await gate!.future;
    outlineReads.add(sessionId);
    return outlines[sessionId] ?? outlineOf(const []);
  }
}

String rawTitle(AgentSession s) => s.title;

Future<void> sweep(
  AgentSearchIndex index,
  FakeReader reader, {
  bool outlines = true,
}) => index.sync(reader, outlines: outlines, chatTitle: rawTitle);

Map<String, dynamic> decode(String target) =>
    jsonDecode(target) as Map<String, dynamic>;

void main() {
  final lisbon = chapter(
    'm1',
    'Lisbon trip: Hotels and transit',
    summary: 'Booked a hotel. Kept the train.',
    sections: [
      section('m2', 'Lisbon trip: Train booking', summary: 'Train at nine.'),
      section('m3', 'Packing list'),
    ],
  );

  group('agentSearchRows', () {
    test('labels chat › chapter, and add a section only for section rows', () {
      final rows = agentSearchRows(
        session: session('s1', 'Trip planning'),
        chatTitle: 'Trip planning',
        outline: outlineOf([lisbon]),
      );
      final byId = {for (final r in rows) r.id: r};
      expect(byId.keys.toList(), [
        'chat:s1',
        'chapter:s1:m1',
        'section:s1:m1:m2',
        'section:s1:m1:m3',
      ]);
      expect(byId['chat:s1']!.title, 'Trip planning');
      expect(byId['chapter:s1:m1']!.title, 'Trip planning › Lisbon trip');
      // The chapter's name prefix is dropped from the section's name.
      expect(
        byId['section:s1:m1:m2']!.title,
        'Trip planning › Lisbon trip › Train booking',
      );
      expect(
        byId['section:s1:m1:m3']!.title,
        'Trip planning › Lisbon trip › Packing list',
      );
    });

    test('bodies are summaries, and no label carries a number', () {
      final rows = agentSearchRows(
        session: session('s1', 'Trip'),
        chatTitle: 'Trip',
        outline: outlineOf([lisbon]),
      );
      expect(rows[1].body, 'Booked a hotel. Kept the train.');
      expect(rows[2].body, 'Train at nine.');
      expect(rows.every((r) => !RegExp(r'\d').hasMatch(r.title)), isTrue);
      expect(rows.first.body, isEmpty);
    });

    test('targets carry anchors and levels; chat rows have empty anchors', () {
      final rows = agentSearchRows(
        session: session('s1', 'Trip'),
        chatTitle: 'Trip',
        outline: outlineOf([lisbon]),
      );
      final chat = decode(rows[0].target);
      expect(chat, {
        'sessionId': 's1',
        'firstMessageId': '',
        'lastMessageId': '',
        'level': 'chat',
      });
      final chap = decode(rows[1].target);
      expect(chap['level'], 'chapter');
      expect(chap['firstMessageId'], 'm1');
      expect(chap['lastMessageId'], 'm1-end');
      expect(chap['chapterTitle'], 'Lisbon trip: Hotels and transit');
      expect(chap.containsKey('sectionTitle'), isFalse);
      final sec = decode(rows[2].target);
      expect(sec['level'], 'section');
      expect(sec['firstMessageId'], 'm2');
      expect(sec['sectionTitle'], 'Lisbon trip: Train booking');
    });

    test('a hub chat without an outline is a title row only', () {
      final rows = agentSearchRows(
        session: session('h1', 'Hub chat'),
        chatTitle: 'Hub chat',
        outline: null,
      );
      expect(rows.map((r) => r.id).toList(), ['chat:h1']);
    });

    test('ids follow the anchors, so a rename keeps them', () {
      final before = agentSearchRows(
        session: session('s1', 'Old'),
        chatTitle: 'Old',
        outline: outlineOf([lisbon]),
      );
      final after = agentSearchRows(
        session: session('s1', 'New'),
        chatTitle: 'New',
        outline: outlineOf([lisbon]),
      );
      expect(after.map((r) => r.id).toList(), before.map((r) => r.id).toList());
      expect(before.every((r) => !r.id.contains('Old')), isTrue);
    });
  });

  group('AgentSearchIndex', () {
    test('writes every chat and reads only chats that changed', () async {
      final sink = FakeSink();
      final reader = FakeReader()
        ..listed = [session('s1', 'Trip'), session('s2', 'Other')]
        ..outlines['s1'] = outlineOf([lisbon]);
      final index = AgentSearchIndex(sink);
      await sweep(index, reader);
      expect(sink.ids, containsAll(['chat:s1', 'chapter:s1:m1', 'chat:s2']));
      expect(reader.outlineReads, ['s1', 's2']);

      reader.outlineReads.clear();
      await sweep(index, reader);
      expect(reader.outlineReads, isEmpty, reason: 'nothing changed');
    });

    test(
      'a changed revision rewrites the chat and drops removed sections',
      () async {
        final sink = FakeSink();
        final reader = FakeReader()
          ..listed = [session('s1', 'Trip')]
          ..outlines['s1'] = outlineOf([lisbon])
          ..revisions['s1'] = BigInt.one;
        final index = AgentSearchIndex(sink);
        await sweep(index, reader);
        expect(sink.rows.containsKey('section:s1:m1:m3'), isTrue);

        reader
          ..revisions['s1'] = BigInt.two
          ..outlines['s1'] = outlineOf([
            chapter(
              'm1',
              'Lisbon trip: Hotels and transit',
              sections: [section('m2', 'Lisbon trip: Train booking')],
            ),
          ]);
        await sweep(index, reader);
        expect(sink.rows.containsKey('section:s1:m1:m3'), isFalse);
        expect(sink.deleted, contains('section:s1:m1:m3'));
        expect(sink.rows.containsKey('section:s1:m1:m2'), isTrue);
      },
    );

    test(
      'a failed delete keeps its id until a later sweep removes the row',
      () async {
        final sink = FakeSink();
        final reader = FakeReader()
          ..listed = [session('s1', 'Trip'), session('s2', 'Doomed')]
          ..outlines['s1'] = outlineOf([lisbon])
          ..outlines['s2'] = outlineOf([chapter('x1', 'Doomed: Only chapter')]);
        final index = AgentSearchIndex(sink);
        await sweep(index, reader);

        reader.listed = [session('s1', 'Trip')];
        sink.failDeletes = true;
        await sweep(index, reader);
        expect(sink.ids.where((id) => id.contains('s2')), isNotEmpty);

        sink.failDeletes = false;
        await sweep(index, reader);
        expect(sink.ids.where((id) => id.contains('s2')), isEmpty);
        expect(sink.rows.containsKey('chat:s1'), isTrue);
      },
    );

    test(
      'a removed chat whose delete failed is deleted by a later sweep',
      () async {
        final sink = FakeSink();
        final reader = FakeReader()
          ..listed = [session('s1', 'Trip'), session('s2', 'Doomed')]
          ..outlines['s1'] = outlineOf([lisbon])
          ..outlines['s2'] = outlineOf([chapter('x1', 'Doomed: Only chapter')]);
        final index = AgentSearchIndex(sink);
        await sweep(index, reader);

        sink.failDeletes = true;
        index.removeSession('s2');
        await sweep(index, reader);
        expect(sink.ids.where((id) => id.contains('s2')), isNotEmpty);

        sink.failDeletes = false;
        await sweep(index, reader);
        expect(sink.ids.where((id) => id.contains('s2')), isEmpty);
      },
    );

    test(
      'close drains queued deletes and starts no sink call after it',
      () async {
        final sink = FakeSink();
        final reader = FakeReader()
          ..listed = [session('s1', 'Trip'), session('s2', 'Doomed')]
          ..outlines['s1'] = outlineOf([lisbon])
          ..outlines['s2'] = outlineOf([chapter('x1', 'Doomed: Only chapter')]);
        final index = AgentSearchIndex(sink);
        await sweep(index, reader);

        index.removeSession('s2');
        await index.close();
        expect(sink.ids.where((id) => id.contains('s2')), isEmpty);

        final calls = sink.calls;
        reader.listed = [session('s3', 'After close')];
        reader.revisions['s1'] = BigInt.one;
        await sweep(index, reader);
        index.removeSession('s1');
        await index.close();
        expect(sink.calls, calls);
        expect(sink.ids.where((id) => id.contains('s1')), isNotEmpty);
      },
    );

    test(
      'close waits for a running sweep and that sweep writes nothing',
      () async {
        final sink = FakeSink();
        final reader = FakeReader()
          ..listed = [session('s1', 'Trip'), session('s2', 'Other')]
          ..outlines['s1'] = outlineOf([lisbon]);
        final index = AgentSearchIndex(sink);
        await sweep(index, reader);

        reader
          ..revisions['s1'] = BigInt.one
          ..revisions['s2'] = BigInt.one
          ..gated = 's2'
          ..gate = Completer<void>();
        final before = sink.calls;
        final running = sweep(index, reader);
        // Let the sweep reach the gated outline read of s2.
        await Future<void>.delayed(Duration.zero);
        await Future<void>.delayed(Duration.zero);

        final closing = index.close();
        reader.gate!.complete();
        await running;
        await closing;
        expect(sink.calls, before);
      },
    );

    test('a rename updates titles in place without deleting rows', () async {
      final sink = FakeSink();
      final reader = FakeReader()
        ..listed = [session('s1', 'Trip')]
        ..outlines['s1'] = outlineOf([lisbon]);
      final index = AgentSearchIndex(sink);
      await sweep(index, reader);
      final ids = sink.ids.toSet();

      reader.listed = [
        session('s1', 'Renamed trip', updated: '2026-10-10T11:00:00Z'),
      ];
      await sweep(index, reader);
      expect(sink.ids.toSet(), ids);
      expect(sink.rows['chat:s1']!.title, 'Renamed trip');
      expect(
        sink.rows['section:s1:m1:m2']!.title,
        'Renamed trip › Lisbon trip › Train booking',
      );
      expect(sink.deleted, isEmpty);
    });

    test(
      'chats that left the list lose their rows at the next sweep',
      () async {
        final sink = FakeSink();
        final reader = FakeReader()
          ..listed = [session('s1', 'Trip'), session('s2', 'Other')]
          ..outlines['s1'] = outlineOf([lisbon]);
        final index = AgentSearchIndex(sink);
        await sweep(index, reader);

        reader.listed = [session('s1', 'Trip')];
        await sweep(index, reader);
        expect(sink.ids.where((id) => id.contains('s2')), isEmpty);
        expect(sink.rows.containsKey('chat:s1'), isTrue);
      },
    );

    test(
      'removeSession drops rows, and an in-flight sweep cannot write them back',
      () async {
        final sink = FakeSink();
        final reader = FakeReader()
          ..listed = [session('s1', 'Trip'), session('s2', 'Doomed')]
          ..outlines['s1'] = outlineOf([lisbon])
          ..outlines['s2'] = outlineOf([chapter('x1', 'Doomed: Only chapter')])
          ..gated = 's2'
          ..gate = Completer<void>();
        final index = AgentSearchIndex(sink);
        final running = sweep(index, reader);
        // Let the sweep reach the gated outline read of s2.
        await Future<void>.delayed(Duration.zero);
        await Future<void>.delayed(Duration.zero);

        reader.listed = [session('s1', 'Trip')];
        index.removeSession('s2');
        reader.gate!.complete();
        await running;
        await sweep(index, reader);

        expect(sink.ids.where((id) => id.contains('s2')), isEmpty);
        expect(sink.ids.where((id) => id.contains('x1')), isEmpty);
        expect(sink.rows.containsKey('chat:s1'), isTrue);
      },
    );

    test('calls during a sweep coalesce; the last arguments win', () async {
      final sink = FakeSink();
      final reader = FakeReader()
        ..listed = [session('s1', 'Old')]
        ..outlines['s1'] = outlineOf([lisbon])
        ..gated = 's1'
        ..gate = Completer<void>();
      final index = AgentSearchIndex(sink);
      final first = sweep(index, reader);
      await Future<void>.delayed(Duration.zero);

      reader.listed = [session('s1', 'New', updated: '2026-10-10T11:00:00Z')];
      await sweep(index, reader);
      reader.gate!.complete();
      await first;
      expect(sink.rows['chat:s1']!.title, 'New');
    });

    test('offline keeps the cached rows and clears nothing', () async {
      final sink = FakeSink();
      final reader = FakeReader()
        ..listed = [session('s1', 'Trip')]
        ..outlines['s1'] = outlineOf([lisbon]);
      final index = AgentSearchIndex(sink);
      await sweep(index, reader);
      final before = sink.ids.toSet();

      reader.offline = true;
      await sweep(index, reader);
      expect(sink.ids.toSet(), before);
      expect(sink.clears, 1, reason: 'only the first complete sweep clears');
    });

    test(
      'the first complete sweep of a process clears stale rows once',
      () async {
        final sink = FakeSink()
          ..rows['chat:gone'] = const AgentSearchRow(
            id: 'chat:gone',
            title: 'Deleted last run',
            body: '',
            target: '{}',
            updatedMs: 0,
          );
        final reader = FakeReader()..listed = [session('s1', 'Trip')];
        final index = AgentSearchIndex(sink);
        await sweep(index, reader);
        expect(sink.rows.containsKey('chat:gone'), isFalse);
        expect(sink.rows.containsKey('chat:s1'), isTrue);
        expect(sink.clears, 1);

        await sweep(index, reader);
        expect(sink.clears, 1);
      },
    );

    test('a sweep that could not read every chat clears nothing', () async {
      final sink = FakeSink()
        ..rows['chat:gone'] = const AgentSearchRow(
          id: 'chat:gone',
          title: 'Stale',
          body: '',
          target: '{}',
          updatedMs: 0,
        );
      final reader = FakeReader()
        ..listed = [session('s1', 'Trip'), session('s2', 'Broken')]
        ..failing.add('s2');
      final index = AgentSearchIndex(sink);
      await sweep(index, reader);
      expect(sink.clears, 0);
      expect(sink.rows.containsKey('chat:gone'), isTrue);
      expect(sink.rows.containsKey('chat:s1'), isTrue);

      reader.failing.clear();
      await sweep(index, reader);
      expect(sink.clears, 1);
      expect(sink.rows.containsKey('chat:gone'), isFalse);
    });

    test('hub chats are indexed by title only', () async {
      final sink = FakeSink();
      final reader = FakeReader()..listed = [session('h1', 'Hub chat')];
      final index = AgentSearchIndex(sink);
      await sweep(index, reader, outlines: false);
      expect(sink.ids.toList(), ['chat:h1']);
      expect(reader.outlineReads, isEmpty);
    });
  });
}
