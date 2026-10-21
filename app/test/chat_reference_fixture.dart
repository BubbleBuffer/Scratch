// A chat outline for the chat reference tests: one closed chapter with two
// sections. Built with the generated outline types (decisions as lists, dates
// as RFC 3339 strings, ongoing as a bool).

import 'package:notes_app/src/rust/api/agent.dart';

const chatRefChatTitle = 'BubblePlay cursor';

OutlineChapter chatRefChapter() => OutlineChapter(
  id: 'ch-2',
  title: 'Native drag-and-drop',
  summary: 'Virtual-file drag from the web to the PC: built, deployed.',
  decisions: const ['Ship for Chrome first; Explorer stays behind a flag.'],
  ongoing: false,
  startedAt: '2026-09-02T10:00:00Z',
  endedAt: '2026-09-04T16:30:00Z',
  firstMessageId: 'm-40',
  lastMessageId: 'm-88',
  sections: [
    OutlineSection(
      id: 's-1',
      title: 'Web to PC path and the handoff stall',
      summary: 'The handoff stalls.',
      decisions: const [],
      firstMessageId: 'm-40',
      lastMessageId: 'm-52',
    ),
    OutlineSection(
      id: 's-2',
      title: 'Chrome live drops pass, Explorer still blocked',
      summary: 'Drops from Chrome work.',
      decisions: const ['Keep Explorer behind a flag.'],
      firstMessageId: 'm-53',
      lastMessageId: 'm-88',
    ),
  ],
);
