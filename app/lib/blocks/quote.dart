// Quote: a bar on the left.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../registry/block_registry.dart';
import '../ui/ui.dart';

Widget _decorate(BlockContext b, Widget text) => Container(
      decoration: BoxDecoration(border: Border(left: BorderSide(color: b.colors.textPrimary, width: 3))),
      padding: const EdgeInsets.only(left: 14),
      child: text,
    );

const quoteBlock = BlockType(
  kind: Kinds.quote,
  label: 'Quote',
  hint: 'Capture a quote.',
  icon: Ico.quote,
  aliases: 'quote blockquote',
  shortcut: '"',
  markdown: ['" '],
  exitOnEmptyEnter: true,
  placeholder: 'Empty quote',
  decorate: _decorate,
);
