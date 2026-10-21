// The built-in block types, in `/` menu order. A new block type is a new
// file in this folder plus one line here.

import '../registry/block_registry.dart';
import 'bookmark.dart';
import 'bulleted.dart';
import 'callout.dart';
import 'code.dart';
import 'columns.dart';
import 'divider.dart';
import 'file.dart';
import 'heading.dart';
import 'image.dart';
import 'numbered.dart';
import 'page_link.dart';
import 'paragraph.dart';
import 'quote.dart';
import 'table.dart';
import 'toc.dart';
import 'todo.dart';
import 'toggle.dart';

void registerBuiltinBlocks(BlockRegistry r) {
  for (final t in const [
    paragraphBlock,
    subPageEntry,
    todoBlock,
    heading1Block,
    heading2Block,
    heading3Block,
    bulletedBlock,
    numberedBlock,
    calloutBlock,
    quoteBlock,
    dividerBlock,
    toggleBlock,
    toggleHeading1Block,
    toggleHeading2Block,
    toggleHeading3Block,
    linkToPageBlock,
    codeBlock,
    tableBlock,
    imageBlock,
    fileBlock,
    bookmarkBlock,
    tocBlock,
    columns2Entry,
    columns3Entry,
    columnListBlock,
    columnBlock,
  ]) {
    r.register(t);
  }
}
