// Text: the default block.

import '../core/models.dart';
import '../registry/block_registry.dart';
import '../ui/ui.dart';

const paragraphBlock = BlockType(
  kind: Kinds.paragraph,
  label: 'Text',
  hint: 'Just start writing with plain text.',
  icon: Ico.text,
  aliases: 'text paragraph plain',
  placeholder: "Write, press '/' for commands…",
);
