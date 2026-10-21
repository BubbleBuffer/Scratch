// Agent search rows through the real store: the native agent, outline writes,
// rename and delete. Most tests record rows in a fake sink; the real-vault
// tests read them back through the vault's own index.
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/src/rust/api/agent.dart';
import 'package:notes_app/src/rust/api/types.dart';

import 'agent_test.dart' show openAgentStore, openAgentScreen, pumpUntil;
import 'harness.dart';
import 'sync_helpers.dart';

class RecordingSink implements AgentSearchSink {
  final rows = <String, AgentSearchRow>{};

  @override
  Future<void> upsert(AgentSearchRow row) async => rows[row.id] = row;

  @override
  Future<void> delete(String id) async => rows.remove(id);

  @override
  Future<void> clear() async => rows.clear();

  /// Rows for [sessionId] at [level], by target.
  List<AgentSearchRow> rowsOf(String sessionId, {String? level}) =>
      rows.values.where((r) {
        final t = jsonDecode(r.target) as Map<String, dynamic>;
        return t['sessionId'] == sessionId &&
            (level == null || t['level'] == level);
      }).toList();
}

/// The open native agent, read the way the store reads it.
class NativeReader implements AgentSearchReader {
  NativeReader(this.agent);

  final NotesAgent agent;

  @override
  Future<List<AgentSession>> sessions() => agent.sessions();

  @override
  Future<BigInt> revision(String sessionId) =>
      agent.sessionOutlineRevision(sessionId: sessionId);

  @override
  Future<ChatOutline> outline(String sessionId) =>
      agent.sessionOutline(sessionId: sessionId);
}

/// The session a row's target names, or null.
String? _sessionOf(String? target) =>
    (jsonDecode(target ?? '{}') as Map<String, dynamic>)['sessionId']
        as String?;

/// The agent rows of [sessionId] that the real index returns for [query],
/// polled until [done] accepts them or about ten seconds pass.
Future<List<Hit>> _indexHits(
  WidgetTester tester,
  String query,
  String sessionId,
  bool Function(List<Hit> hits) done,
) async {
  var mine = <Hit>[];
  for (var i = 0; i < 200; i++) {
    await real(tester, () => current.vault.flush());
    final results = await real(
      tester,
      () =>
          current.vault.run((v) => v.searchAdaptive(query: query, limit: 100)),
    );
    mine = results.hits
        .where(
          (h) =>
              h.source == agentSearchSource &&
              _sessionOf(h.target) == sessionId,
        )
        .toList();
    if (done(mine)) break;
    await real(
      tester,
      () => Future<void>.delayed(const Duration(milliseconds: 50)),
    );
  }
  return mine;
}

/// Waits until the open chat's cached session title is the title the agent
/// stores now, and its section row is indexed under that same title. The
/// cache only reloads when the outline poll sees a change or the chat is
/// reopened, so the chat is reopened until both sides agree. The label is
/// never read from the cache alone.
Future<void> _settledSectionRow(
  WidgetTester tester,
  AgentStore store,
  RecordingSink sink,
  String id,
  String sectionId,
) async {
  for (var i = 0; i < 200; i++) {
    await real(tester, store.refreshSessions);
    final fresh = [
      for (final s in store.sessions)
        if (s.id == id) s.title,
    ];
    final cached = store.state?.session;
    if (fresh.isNotEmpty && cached?.id == id && cached!.title == fresh.first) {
      final row = sink.rows[sectionId];
      if (row != null &&
          row.title ==
              '${store.displaySessionTitle(id, fresh.first)} › '
                  'Lisbon trip › Train booking') {
        return;
      }
    } else {
      await real(tester, () => store.openSession(null));
      await real(tester, () => store.openSession(id));
    }
    await real(
      tester,
      () => Future<void>.delayed(const Duration(milliseconds: 50)),
    );
  }
  fail('the section row never settled under the chat\'s current title');
}

void main() {
  setUpAll(initCore);

  appTest(
    'chat, chapter and section rows follow outline writes, a rename and a delete',
    (tester) async {
      final sink = RecordingSink();
      final previous = AgentStore.searchSinkFor;
      AgentStore.searchSinkFor = (_) => sink;
      addTearDown(() => AgentStore.searchSinkFor = previous);
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession(title: 'Trip planning'));
      await openAgentScreen(tester);
      await real(tester, () => store.send('Say ok'));
      await settleUntil(
        tester,
        () =>
            !store.running &&
            (store.state?.messages.any((m) => m.role == 'assistant') ?? false),
      );
      final id = store.openId!;
      final agent = (await real(tester, store.ensureOpen))!;
      final first = store.state!.messages.first;
      final last = store.state!.messages.last;
      await real(
        tester,
        () => agent.upsertOutlineChapter(
          sessionId: id,
          chapter: OutlineChapter(
            id: 'lisbon',
            title: 'Lisbon trip: Hotels and transit',
            summary: 'Booked a hotel.',
            decisions: const [],
            ongoing: true,
            firstMessageId: first.id,
            lastMessageId: last.id,
            startedAt: first.createdAt,
            sections: const [],
          ),
        ),
      );
      await real(
        tester,
        () => agent.upsertOutlineSection(
          sessionId: id,
          chapterFirstMessageId: first.id,
          section: OutlineSection(
            id: 'train',
            title: 'Lisbon trip: Train booking',
            summary: 'Train at nine.',
            decisions: const [],
            firstMessageId: first.id,
            lastMessageId: last.id,
          ),
        ),
      );
      await pumpUntil(
        tester,
        () => sink.rowsOf(id, level: 'section').isNotEmpty,
        diagnostics: () => '${sink.rows.keys}',
      );
      await _settledSectionRow(
        tester,
        store,
        sink,
        id,
        'section:$id:${first.id}:${first.id}',
      );
      final levels = {for (final r in sink.rowsOf(id)) r.id.split(':').first};
      expect(levels, {'chat', 'chapter', 'section'});
      // An untitled chat takes its newest chapter's title, so its shown title
      // is that chapter's name.
      final shown = store.displaySessionTitle(id, store.state!.session.title);
      expect(
        sink.rows['section:$id:${first.id}:${first.id}']!.title,
        '$shown › Lisbon trip › Train booking',
      );
      expect(sink.rowsOf(id, level: 'section').single.body, 'Train at nine.');

      await real(tester, () => store.renameSession(id, 'Renamed trip'));
      // The sweep writes the chat row before its chapter rows, so wait for both.
      await pumpUntil(
        tester,
        () =>
            sink.rows['chat:$id']?.title == 'Renamed trip' &&
            sink.rows['chapter:$id:${first.id}']?.title ==
                'Renamed trip › Lisbon trip',
      );
      expect(
        sink.rows['chapter:$id:${first.id}']!.title,
        'Renamed trip › Lisbon trip',
      );

      await real(tester, () => store.deleteSession(id));
      await pumpUntil(tester, () => sink.rowsOf(id).isEmpty);
      expect(tester.takeException(), isNull);
    },
  );

  appTest(
    'replay outline rows follow the short fixture as it grows, renames and deletes',
    (tester) async {
      final sink = RecordingSink();
      final previous = AgentStore.searchSinkFor;
      AgentStore.searchSinkFor = (_) => sink;
      addTearDown(() => AgentStore.searchSinkFor = previous);
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await openAgentScreen(tester);
      final playback = await real(
        tester,
        () => store.startReplay(
          File('test/fixtures/chat_live_replay.sqlite').absolute.path,
          delayMs: 100,
        ),
      );
      final id = playback.sessionId;
      await pumpUntil(
        tester,
        () => sink.rowsOf(id, level: 'section').isNotEmpty,
        diagnostics: () => '${playback.error}; ${store.error}',
      );
      await pumpUntil(
        tester,
        () => playback.finished || playback.error != null,
        diagnostics: () => '${playback.error}; ${playback.sent}',
      );
      expect(playback.error, isNull);
      // The fixture ends with two chapters and three sections, as the live
      // replay test asserts.
      await pumpUntil(
        tester,
        () =>
            sink.rowsOf(id, level: 'chapter').length == 2 &&
            sink.rowsOf(id, level: 'section').length == 3,
        diagnostics: () => '${sink.rows.values.map((r) => r.title).toList()}',
      );
      final shown = store.displaySessionTitle(id, store.state!.session.title);
      expect(sink.rows['chat:$id']!.title, shown);

      await real(tester, () => store.renameSession(id, 'Renamed replay'));
      await pumpUntil(
        tester,
        () =>
            sink.rows['chat:$id']?.title == 'Renamed replay' &&
            sink.rowsOf(id, level: 'chapter').length == 2 &&
            sink
                .rowsOf(id, level: 'chapter')
                .every((r) => r.title.startsWith('Renamed replay › ')),
      );
      expect(
        sink
            .rowsOf(id, level: 'chapter')
            .every((r) => r.title.startsWith('Renamed replay › ')),
        isTrue,
      );

      await real(tester, () => store.deleteSession(id));
      await pumpUntil(tester, () => sink.rowsOf(id).isEmpty);
      expect(tester.takeException(), isNull);
    },
    skip: !devReplayEnabled,
  );

  // The next tests use the production default sink, so their rows go into the
  // real vault index and are read back through it.

  appTest(
    'a platform switch while the agent stays open does not relabel its rows',
    (tester) async {
      final sink = RecordingSink();
      final previous = AgentStore.searchSinkFor;
      AgentStore.searchSinkFor = (_) => sink;
      addTearDown(() => AgentStore.searchSinkFor = previous);
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession(title: 'Trip planning'));
      await openAgentScreen(tester);
      await real(tester, () => store.send('Say ok'));
      await settleUntil(
        tester,
        () =>
            !store.running &&
            (store.state?.messages.any((m) => m.role == 'assistant') ?? false),
      );
      final id = store.openId!;
      final agent = (await real(tester, store.ensureOpen))!;
      final first = store.state!.messages.first;
      final last = store.state!.messages.last;
      await real(
        tester,
        () => agent.upsertOutlineChapter(
          sessionId: id,
          chapter: OutlineChapter(
            id: 'trip',
            title: 'Trip planning: Next steps',
            summary: 'Still planning the trip.',
            decisions: const [],
            ongoing: true,
            firstMessageId: first.id,
            lastMessageId: last.id,
            startedAt: first.createdAt,
            sections: const [],
          ),
        ),
      );
      final chapterId = 'chapter:$id:${first.id}';
      await pumpUntil(tester, () => sink.rows[chapterId] != null);
      // The local agent names rows by the gist of the chat title.
      final before = sink.rows[chapterId]!.title;
      expect(before, startsWith('Trip planning › '));

      // The platform now reads as Android, where the store's display title is
      // the raw one. The agent is still the local one, so a sweep must keep
      // the rows as they were. The platform is restored in the body, before
      // appTest's own cleanup runs.
      final restorePlatform = debugDefaultTargetPlatformOverride;
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      try {
        expect(store.host, AgentHost.hub);
        await real(tester, store.refreshSessions);
        // A real change forces an indexed sweep after the switch: the new
        // summary changes the outline revision, and a refresh sweeps it into
        // the chapter row. Waiting for that text proves the sweep ran.
        const summary = 'Summary written after the platform switch';
        await real(
          tester,
          () => agent.upsertOutlineChapter(
            sessionId: id,
            chapter: OutlineChapter(
              id: 'trip',
              title: 'Trip planning: Next steps',
              summary: summary,
              decisions: const [],
              ongoing: true,
              firstMessageId: first.id,
              lastMessageId: last.id,
              startedAt: first.createdAt,
              sections: const [],
            ),
          ),
        );
        await real(tester, store.refreshSessions);
        await pumpUntil(tester, () => sink.rows[chapterId]?.body == summary);
        expect(sink.rows[chapterId]!.title, before);
        expect(sink.rows['chat:$id']!.title, isNot(contains(':')));
        expect(tester.takeException(), isNull);
      } finally {
        debugDefaultTargetPlatformOverride = restorePlatform;
      }
    },
  );

  appTest('the production sink writes native chat rows into the vault index', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    expect(
      AgentStore.searchSinkFor!(current.vault),
      isA<VaultAgentSearchSink>(),
    );
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Trip planning'));
    await openAgentScreen(tester);
    await real(tester, () => store.send('Say ok'));
    await settleUntil(
      tester,
      () =>
          !store.running &&
          (store.state?.messages.any((m) => m.role == 'assistant') ?? false),
    );
    final id = store.openId!;
    final agent = (await real(tester, store.ensureOpen))!;
    final first = store.state!.messages.first;
    final last = store.state!.messages.last;
    await real(
      tester,
      () => agent.upsertOutlineChapter(
        sessionId: id,
        chapter: OutlineChapter(
          id: 'lisbon',
          title: 'Lisbon trip: Hotels and transit',
          summary: 'Booked a hotel.',
          decisions: const [],
          ongoing: true,
          firstMessageId: first.id,
          lastMessageId: last.id,
          startedAt: first.createdAt,
          sections: const [],
        ),
      ),
    );
    await real(
      tester,
      () => agent.upsertOutlineSection(
        sessionId: id,
        chapterFirstMessageId: first.id,
        section: OutlineSection(
          id: 'train',
          title: 'Lisbon trip: Train booking',
          summary: 'Train at nine.',
          decisions: const [],
          firstMessageId: first.id,
          lastMessageId: last.id,
        ),
      ),
    );
    final section = await _indexHits(
      tester,
      'Train booking',
      id,
      (hits) => hits.any((h) => h.title.contains('Train booking')),
    );
    await real(tester, store.refreshSessions);
    final shown = store.displaySessionTitle(
      id,
      store.sessions.firstWhere((session) => session.id == id).title,
    );
    expect(
      section.map((h) => h.title),
      contains('$shown › Lisbon trip › Train booking'),
    );
    expect(section.every((h) => h.pageType == agentSearchPageType), isTrue);

    await real(tester, () => store.renameSession(id, 'Renamed trip'));
    final renamed = await _indexHits(
      tester,
      'Renamed trip',
      id,
      (hits) =>
          hits.any((h) => h.title == 'Renamed trip') &&
          hits.any((h) => h.title == 'Renamed trip › Lisbon trip'),
    );
    expect(renamed.map((h) => h.title), contains('Renamed trip'));
    expect(renamed.map((h) => h.title), contains('Renamed trip › Lisbon trip'));

    await real(tester, () => store.deleteSession(id));
    final gone = await _indexHits(
      tester,
      'Renamed trip',
      id,
      (hits) => hits.isEmpty,
    );
    expect(gone, isEmpty);
    expect(tester.takeException(), isNull);
  });

  appTest(
    'a fresh index clears a stale agent row at its first sweep and keeps live chats',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession(title: 'Keep me'));
      final keep = store.openId!;
      await _indexHits(
        tester,
        'Keep me',
        keep,
        (hits) => hits.any((h) => h.title == 'Keep me'),
      );

      // A row for a chat that no longer exists, as an earlier run would leave.
      await real(
        tester,
        () => VaultAgentSearchSink(current.vault).upsert(
          const AgentSearchRow(
            id: 'chat:gone',
            title: 'Gone chat',
            body: '',
            target: '{"sessionId":"gone","level":"chat"}',
            updatedMs: 0,
          ),
        ),
      );
      final stale = await _indexHits(
        tester,
        'Gone chat',
        'gone',
        (hits) => hits.isNotEmpty,
      );
      expect(stale, isNotEmpty);

      // A new index is what a restarted process builds. Its first complete
      // sweep clears the source, then writes the live chats again. The store's
      // own index is left alone, so no UI is torn down.
      final agent = (await real(tester, store.ensureOpen))!;
      // Construct the queue on the real clock, like native agent opening.
      // A fake-clock initial Future cannot complete inside runAsync.
      final restarted = await real(
        tester,
        () async => AgentSearchIndex(VaultAgentSearchSink(current.vault)),
      );
      await real(
        tester,
        () => restarted.sync(
          NativeReader(agent),
          outlines: true,
          chatTitle: (s) => store.displaySessionTitle(s.id, s.title),
        ),
      );
      expect(restarted.reconciled, isTrue);
      await real(tester, () => restarted.close());

      final gone = await _indexHits(
        tester,
        'Gone chat',
        'gone',
        (hits) => hits.isEmpty,
      );
      expect(gone, isEmpty);
      final kept = await _indexHits(
        tester,
        'Keep me',
        keep,
        (hits) => hits.any((h) => h.title == 'Keep me'),
      );
      expect(kept.map((h) => h.title), contains('Keep me'));
      expect(tester.takeException(), isNull);
    },
  );

  appTest(
    'replay rows reach the real vault index and follow a rename and a delete',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await openAgentScreen(tester);
      final playback = await real(
        tester,
        () => store.startReplay(
          File('test/fixtures/chat_live_replay.sqlite').absolute.path,
          delayMs: 100,
        ),
      );
      final id = playback.sessionId;
      await pumpUntil(
        tester,
        () => playback.finished || playback.error != null,
        diagnostics: () => '${playback.error}; ${playback.sent}',
      );
      expect(playback.error, isNull);
      final shown = store.displaySessionTitle(id, store.state!.session.title);
      final agent = (await real(tester, store.ensureOpen))!;
      final outline = await real(
        tester,
        () => agent.sessionOutline(sessionId: id),
      );
      // Chapter rows carry the chapter name, so search for a word from the
      // first chapter's title.
      final word = RegExp(r'[A-Za-z]{4,}')
          .firstMatch(outline.chapters.first.title)!
          .group(0)!;

      final chats = await _indexHits(
        tester,
        shown,
        id,
        (hits) => hits.any((h) => h.title == shown),
      );
      expect(chats.map((h) => h.title), contains(shown));
      final chapters = await _indexHits(
        tester,
        word,
        id,
        (hits) => hits.any((h) => h.title.startsWith('$shown › ')),
      );
      expect(
        chapters.where((h) => h.title.startsWith('$shown › ')),
        isNotEmpty,
      );

      await real(tester, () => store.renameSession(id, 'Renamed replay'));
      final renamed = await _indexHits(
        tester,
        word,
        id,
        (hits) =>
            hits.any((h) => h.title.startsWith('Renamed replay › ')) &&
            hits
                .where((h) => h.title.contains(' › '))
                .every((h) => h.title.startsWith('Renamed replay › ')),
      );
      expect(
        renamed.any((h) => h.title.startsWith('Renamed replay › ')),
        isTrue,
      );
      expect(
        renamed
            .where((h) => h.title.contains(' › '))
            .every((h) => h.title.startsWith('Renamed replay › ')),
        isTrue,
      );

      await real(tester, () => store.deleteSession(id));
      final gone = await _indexHits(tester, word, id, (hits) => hits.isEmpty);
      expect(gone, isEmpty);
      final goneChat = await _indexHits(
        tester,
        'Renamed replay',
        id,
        (hits) => hits.isEmpty,
      );
      expect(goneChat, isEmpty);
      expect(tester.takeException(), isNull);
    },
    skip: !devReplayEnabled,
  );
}
