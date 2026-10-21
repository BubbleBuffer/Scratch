// Divider: a horizontal line, no text.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../registry/block_registry.dart';
import '../ui/ui.dart';

Widget _body(BlockContext b) => SizedBox(
      height: 13,
      child: Center(child: Container(height: 1, color: b.colors.divider)),
    );

double _handleTop(BlockContext b) => -5.5;

const dividerBlock = BlockType(
  kind: Kinds.divider,
  label: 'Divider',
  hint: 'Visually divide blocks.',
  icon: Ico.divider,
  aliases: 'divider line hr separator',
  shortcut: '---',
  markdown: ['---'],
  editable: false,
  acceptsChildren: false,
  body: _body,
  handleTop: _handleTop,
);
