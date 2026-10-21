import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/src/rust/api/agent.dart';
import 'package:notes_app/ui/ui.dart';

import 'chat_reference_fixture.dart';

class _Source implements ChatReferenceSource {
  @override
  bool enabled = true;
  ChatReferenceSnapshot? snapshot;
  int reads = 0;
  @override
  Future<ChatReferenceSnapshot?> load(String sessionId) async {
    expect(sessionId, 'session-id');
    reads++;
    return snapshot;
  }
}

OutlineChapter _current({String title = 'Current chapter'}) {
  final original = chatRefChapter();
  return OutlineChapter(
    id: 'a-new-segmentation-id',
    title: title,
    summary: 'Current searchable summary.',
    decisions: const ['Current decision.'],
    ongoing: original.ongoing,
    firstMessageId: original.firstMessageId,
    lastMessageId: original.lastMessageId,
    startedAt: original.startedAt,
    endedAt: original.endedAt,
    sections: [
      for (final s in original.sections)
        OutlineSection(
          id: 'new-${s.id}',
          title: 'Current ${s.title}',
          summary: 'Current section summary.',
          decisions: const [],
          firstMessageId: s.firstMessageId,
          lastMessageId: s.lastMessageId,
        ),
    ],
  );
}

ChatReferenceSnapshot _snapshot({String title = 'Current chapter'}) =>
    ChatReferenceSnapshot(
      chatTitle: 'Renamed chat',
      messageIds: [for (var i = 40; i <= 88; i++) 'm-$i'],
      outline: ChatOutline(chapters: [_current(title: title)]),
    );

ChatReference _cached() => ChatReference.fromChapter(
  sessionId: 'session-id',
  chatTitle: chatRefChatTitle,
  chapter: chatRefChapter(),
);

Widget _app(Widget child) => AppTheme(
  colors: AppColors.light,
  formFactor: FormFactor.desktop,
  child: WidgetsApp(
    color: AppColors.light.bgPrimary,
    pageRouteBuilder: <T>(settings, builder) => PageRouteBuilder<T>(
      settings: settings,
      pageBuilder: (context, _, _) => builder(context),
    ),
    home: Center(child: SizedBox(width: 420, child: child)),
  ),
);

Future<void> _finishResolve(WidgetTester tester) async {
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
}

void main() {
  test(
    'renamed chat and retitled chapter resolve by IDs and preserve the range',
    () {
      final cached = _cached();
      final resolved = resolveChatReference(cached, _snapshot())!;
      expect(resolved.chatTitle, 'Renamed chat');
      expect(resolved.chapterTitle, 'Current chapter');
      expect(resolved.summary, 'Current searchable summary.');
      expect(resolved.firstMessageId, cached.firstMessageId);
      expect(resolved.lastMessageId, cached.lastMessageId);
      expect(resolved.sessionId, cached.sessionId);
    },
  );

  test('a retitled section resolves after its segmentation ID changed', () {
    final chapter = chatRefChapter();
    final cached = ChatReference.fromSection(
      sessionId: 'session-id',
      chatTitle: chatRefChatTitle,
      chapter: chapter,
      section: chapter.sections.last,
    );
    final resolved = resolveChatReference(cached, _snapshot())!;
    expect(resolved.chapterTitle, 'Current chapter');
    expect(resolved.sectionTitle, 'Current ${chapter.sections.last.title}');
    expect(resolved.summary, 'Current section summary.');
    expect(resolved.firstMessageId, 'm-53');
    expect(resolved.lastMessageId, 'm-88');
  });

  test(
    'a missing message anchor cannot resolve to an unrelated current range',
    () {
      final snapshot = ChatReferenceSnapshot(
        chatTitle: 'Unrelated chat',
        messageIds: const ['other-first', 'other-last'],
        outline: ChatOutline(chapters: [_current()]),
      );
      expect(resolveChatReference(_cached(), snapshot), isNull);
    },
  );

  for (final chip in [false, true]) {
    testWidgets(
      '${chip ? 'chip' : 'card'} refreshes names once and observes a retitle',
      (tester) async {
        final source = _Source()..snapshot = _snapshot();
        final writes = <ChatReference>[];
        final cached = _cached();
        final child = chip
            ? ChatReferenceChip(
                reference: cached,
                source: source,
                onCacheChanged: writes.add,
              )
            : ChatReferenceCard(
                reference: cached,
                source: source,
                onCacheChanged: writes.add,
              );
        await tester.pumpWidget(_app(child));
        await _finishResolve(tester);
        expect(find.textContaining('Renamed chat'), findsOneWidget);
        expect(writes, hasLength(1));
        expect(writes.single.summary, 'Current searchable summary.');
        await tester.pump(const Duration(seconds: 1));
        await _finishResolve(tester);
        expect(
          writes,
          hasLength(1),
          reason: 'unchanged data never writes on each render',
        );
        source.snapshot = _snapshot(title: 'Retitled again');
        await tester.pump(const Duration(seconds: 1));
        await _finishResolve(tester);
        expect(find.textContaining('Retitled again'), findsWidgets);
        expect(writes, hasLength(2));
        expect(writes.last.firstMessageId, cached.firstMessageId);
        expect(writes.last.lastMessageId, cached.lastMessageId);
        await tester.pumpWidget(const SizedBox.shrink());
      },
    );

    testWidgets(
      '${chip ? 'chip' : 'card'} keeps its cache for a deleted chat with a muted notice',
      (tester) async {
        final source = _Source();
        final writes = <ChatReference>[];
        final child = chip
            ? ChatReferenceChip(
                reference: _cached(),
                source: source,
                onCacheChanged: writes.add,
              )
            : ChatReferenceCard(
                reference: _cached(),
                source: source,
                onCacheChanged: writes.add,
              );
        await tester.pumpWidget(_app(child));
        await _finishResolve(tester);
        expect(find.textContaining(chatRefChatTitle), findsOneWidget);
        expect(find.text('chat unavailable'), findsOneWidget);
        expect(writes, isEmpty);
        await tester.pumpWidget(const SizedBox.shrink());
      },
    );

    testWidgets(
      '${chip ? 'chip' : 'card'} renders from cache with the agent off without reads or writes',
      (tester) async {
        final source = _Source()..enabled = false;
        final writes = <ChatReference>[];
        final child = chip
            ? ChatReferenceChip(
                reference: _cached(),
                source: source,
                onCacheChanged: writes.add,
              )
            : ChatReferenceCard(
                reference: _cached(),
                source: source,
                onCacheChanged: writes.add,
              );
        await tester.pumpWidget(_app(child));
        await _finishResolve(tester);
        expect(find.textContaining(chatRefChatTitle), findsOneWidget);
        expect(find.text('chat unavailable'), findsNothing);
        expect(source.reads, 0);
        expect(writes, isEmpty);
        await tester.pumpWidget(const SizedBox.shrink());
      },
    );
  }
}
