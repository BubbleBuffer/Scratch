// Golden screenshots of the chat reference card: folded and open, a chapter
// (stacked sheets) and a section, light and dark, plus the inline chip.
// Regenerate with: flutter test --update-goldens test/chat_reference_golden_test.dart

@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/ui/ui.dart';

import 'chat_reference_fixture.dart';
import 'harness.dart';

// The card reads MediaQuery and the theme, so both are supplied here.
Widget themed(Widget child, {required bool dark}) => MediaQuery(
  data: const MediaQueryData(size: Size(600, 800)),
  child: AppTheme(
    colors: dark ? AppColors.darkTheme : AppColors.light,
    formFactor: FormFactor.desktop,
    child: Directionality(
      textDirection: TextDirection.ltr,
      child: Center(child: SizedBox(width: 560, child: child)),
    ),
  ),
);

ChatReference _chapterRef() => ChatReference.fromChapter(
  sessionId: 'sess-1',
  chatTitle: chatRefChatTitle,
  chapter: chatRefChapter(),
);

ChatReference _sectionRef() {
  final chapter = chatRefChapter();
  return ChatReference.fromSection(
    sessionId: 'sess-1',
    chatTitle: chatRefChatTitle,
    chapter: chapter,
    section: chapter.sections.last,
  );
}

void main() {
  // Cards and chips need the bundled fonts, not the native vault.
  setUpAll(loadFonts);

  for (final dark in [false, true]) {
    final tag = dark ? 'dark' : 'light';

    testWidgets('chapter card folded $tag', (tester) async {
      await tester.pumpWidget(
        themed(ChatReferenceCard(reference: _chapterRef()), dark: dark),
      );
      await tester.pump(const Duration(milliseconds: 400));
      await expectLater(
        find.byType(ChatReferenceCard),
        matchesGoldenFile('goldens/chat-ref-chapter-folded-$tag.png'),
      );
    });

    testWidgets('chapter card open $tag', (tester) async {
      await tester.pumpWidget(
        themed(
          ChatReferenceCard(
            reference: _chapterRef(),
            onOpenInChat: () {},
            initiallyOpen: true,
          ),
          dark: dark,
        ),
      );
      await tester.pump(const Duration(milliseconds: 400));
      await expectLater(
        find.byType(ChatReferenceCard),
        matchesGoldenFile('goldens/chat-ref-chapter-open-$tag.png'),
      );
    });

    testWidgets('section card folded $tag', (tester) async {
      await tester.pumpWidget(
        themed(ChatReferenceCard(reference: _sectionRef()), dark: dark),
      );
      await tester.pump(const Duration(milliseconds: 400));
      await expectLater(
        find.byType(ChatReferenceCard),
        matchesGoldenFile('goldens/chat-ref-section-folded-$tag.png'),
      );
    });

    testWidgets('chip $tag', (tester) async {
      await tester.pumpWidget(
        themed(
          Padding(
            padding: const EdgeInsets.all(12),
            child: ChatReferenceChip(reference: _sectionRef()),
          ),
          dark: dark,
        ),
      );
      await tester.pump(const Duration(milliseconds: 400));
      await expectLater(
        find.byType(ChatReferenceChip),
        matchesGoldenFile('goldens/chat-ref-chip-$tag.png'),
      );
    });
  }

  testWidgets('a folded card opens in place on tap and shows Open in chat', (
    tester,
  ) async {
    var opened = 0;
    await tester.pumpWidget(
      themed(
        ChatReferenceCard(
          reference: _chapterRef(),
          onOpenInChat: () => opened++,
        ),
        dark: false,
      ),
    );
    await tester.pump(const Duration(milliseconds: 300));

    const decision = 'Ship for Chrome first; Explorer stays behind a flag.';
    expect(
      find.byKey(const ValueKey('chatref-open')),
      findsNothing,
      reason: 'folded by default',
    );
    expect(
      find.text(decision),
      findsOneWidget,
      reason: 'the folded peek shows the decision',
    );
    expect(
      find.text('Chrome live drops pass, Explorer still blocked'),
      findsNothing,
      reason: 'sections are behind the fold',
    );

    await tester.tap(find.byKey(const ValueKey('chatref-fold')));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('chatref-open')), findsOneWidget);
    expect(
      find.text('Virtual-file drag from the web to the PC: built, deployed.'),
      findsOneWidget,
    );
    expect(
      find.text('Chrome live drops pass, Explorer still blocked'),
      findsOneWidget,
    );

    await tester.tap(find.byKey(const ValueKey('chatref-open')));
    expect(opened, 1);
  });
}
