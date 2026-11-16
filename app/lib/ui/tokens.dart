// Moved to agent-kit (package:bb_tokens, wiki/app/agent-kit.md);
// this re-export keeps the app's imports unchanged. The app clock stays here,
// and Scratch's own named values (tokens/) come with the shared ones.

export 'package:bb_ui/tokens.dart';

export '../core/clock.dart' show clockNow;
export 'tokens/glyph_tokens.dart';
