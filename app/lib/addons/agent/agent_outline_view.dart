// The outline's parts in the chat: a closed chapter's folding card, the heading
// of an open chapter, section dividers with their menu, the where-am-I strip
// over the transcript and the contents sheet. The chat lays them out (see
// agent_outline.dart) and decides what a tap does. The parts live in
// outline_chapter_widgets.dart, outline_location.dart and
// outline_contents.dart; this file keeps the one public entry point.

export 'outline_chapter_widgets.dart' hide outlineCountLine;
export 'outline_contents.dart';
export 'outline_location.dart';
