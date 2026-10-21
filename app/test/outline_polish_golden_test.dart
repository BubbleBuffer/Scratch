import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/src/rust/api/agent.dart';
import 'package:notes_app/ui/ui.dart';

import 'chat_reference_golden_test.dart' show themed;
import 'harness.dart' show loadFonts;

void main() {
  setUpAll(loadFonts);
  final chapter = OutlineChapter(
    id: 'cursor',
    title: 'BubblePlay headless cursor: Mouse Keys workaround hardened after virtual-mouse driver route dropped',
    summary: 'Mouse Keys provides a visible cursor without an attached mouse. Restore it after disconnect.',
    decisions: const [],
    ongoing: false,
    firstMessageId: 'm1',
    lastMessageId: 'm8',
    startedAt: '2026-10-10T10:00:00Z',
    endedAt: '2026-10-10T11:00:00Z',
    sections: const [
      OutlineSection(
        id: 'keys',
        title: 'BubblePlay headless cursor: Mouse Keys fix, alternatives, virtual mouse driver',
        summary: 'A visible cursor works in a headless session.',
        decisions: [],
        firstMessageId: 'm1',
        lastMessageId: 'm8',
      ),
    ],
  );
  final reference = ChatReference.fromChapter(
    sessionId: 'chat',
    chatTitle: 'BubblePlay headless cursor',
    chapter: chapter,
  );
  for (final dark in [false, true]) {
    final tag = dark ? 'dark' : 'light';
    testWidgets('chat sentence chapter folded $tag', (tester) async {
      await tester.pumpWidget(
        themed(
          OutlineChapterCard(
            chapter: OutlineView.of(ChatOutline(chapters: [chapter]))
                .chapters
                .first,
            color: const Color(0xffdd925b),
            messagesShown: false,
            onToggleMessages: () {},
            onAddToNote: () {},
            onOpenSection: (_) {},
          ),
          dark: dark,
        ),
      );
      await tester.pump(const Duration(milliseconds: 400));
      expect(find.text('BubblePlay headless cursor'), findsOneWidget);
      expect(find.byType(DecisionLine), findsNothing);
      await expectLater(
        find.byType(OutlineChapterCard),
        matchesGoldenFile(
          'goldens/outline-polish/chat-chapter-folded-$tag.png',
        ),
      );
    });
    for (final open in [false, true]) {
      testWidgets('sentence chapter ${open ? 'open' : 'folded'} $tag', (
        tester,
      ) async {
        await tester.pumpWidget(
          themed(
            ChatReferenceCard(
              reference: reference,
              initiallyOpen: open,
              onOpenInChat: () {},
            ),
            dark: dark,
          ),
        );
        await tester.pump(const Duration(milliseconds: 400));
        expect(find.byType(DecisionLine), findsNothing);
        await expectLater(
          find.byType(ChatReferenceCard),
          matchesGoldenFile(
            'goldens/outline-polish/chapter-${open ? 'open' : 'folded'}-$tag.png',
          ),
        );
      });
    }
    testWidgets('sentence section without repeated chapter $tag', (
      tester,
    ) async {
      await tester.pumpWidget(
        themed(
          ChatReferenceCard(
            reference: ChatReference.fromSection(
              sessionId: 'chat',
              chatTitle: reference.chatTitle,
              chapter: chapter,
              section: chapter.sections.first,
            ),
          ),
          dark: dark,
        ),
      );
      await tester.pump(const Duration(milliseconds: 400));
      expect(find.byType(DecisionLine), findsNothing);
      await expectLater(
        find.byType(ChatReferenceCard),
        matchesGoldenFile('goldens/outline-polish/section-folded-$tag.png'),
      );
    });
    testWidgets('full width where strip $tag', (tester) async {
      final where = ValueNotifier<OutlineWhere>((
        chapter: 'BubblePlay headless cursor',
        section: 'Mouse Keys fix',
        chapterId: 'cursor',
        sectionId: 'keys',
      ));
      var taps = 0;
      await tester.pumpWidget(
        themed(
          OutlineWhereStrip(
            where: where,
            colors: const {'cursor': Color(0xffdd925b)},
            onTap: () => taps++,
          ),
          dark: dark,
        ),
      );
      expect(find.text('Contents ▾'), findsOneWidget);
      await tester.tap(find.text('Contents ▾'));
      expect(taps, 1);
      await expectLater(
        find.byType(OutlineWhereStrip),
        matchesGoldenFile('goldens/outline-polish/strip-$tag.png'),
      );
      await tester.pumpWidget(const SizedBox());
      where.dispose();
    });
  }
}
