// Callout: neutral paper with a small space seam by default. Explicit block
// formatting still follows the authored colour. The target is its emoji.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../registry/block_registry.dart';
import '../ui/block_colors.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';
import 'heading.dart';

const defaultCalloutIcon = '💡';

Widget _decorate(BlockContext b, Widget text) {
  final n = b.node;
  final icon = n.target ?? defaultCalloutIcon;
  final plain = n.color == null || n.color == 'default';
  final host = b.host;
  final patch = spacePatch(
    host is HeadingProgressHost
        ? (host as HeadingProgressHost).headingProgressSpaceColor
        : null,
    b.colors.dark ? Brightness.dark : Brightness.light,
  );
  final paper = Container(
    key: ValueKey('callout-${n.id}'),
    decoration: BoxDecoration(
      color: plain ? b.colors.bgPrimary : calloutFill(n.color, b.colors),
      border: plain ? Border.all(color: b.colors.border) : null,
      borderRadius: Radii.panel,
    ),
    padding: EdgeInsets.fromLTRB(
      12,
      b.mobile ? 12 : 16,
      16,
      b.mobile ? 12 : 16,
    ),
    child: Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        // The emoji is centred on the first line of text.
        SizedBox(
          height: 32,
          child: Builder(
            builder: (anchor) => Hover(
              key: ValueKey('callout-icon-${n.id}'),
              borderRadius: Radii.ctl,
              background: b.colors.bgPrimary,
              onTap: () => b.host.openEmojiPicker(n, anchor),
              child: SizedBox(
                width: 32,
                height: 32,
                child: Center(
                  child: Text(
                    icon,
                    style: TextStyle(
                      fontSize: 17,
                      height: 1,
                      fontFamily: sansFamily,
                      fontFamilyFallback: emojiFallback,
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
        const SizedBox(width: 8),
        Expanded(child: text),
      ],
    ),
  );
  if (!plain || patch == null) return paper;
  return Stack(
    children: [
      paper,
      Positioned(
        left: 0,
        top: 14,
        bottom: 14,
        width: 3,
        child: IgnorePointer(
          child: DecoratedBox(
            decoration: BoxDecoration(
              color: patch.fill,
              borderRadius: BorderRadius.circular(2),
            ),
          ),
        ),
      ),
    ],
  );
}

double _handleTop(BlockContext b) =>
    (b.mobile ? 3 : 2) + (b.mobile ? 12 : 16) + (32 - 24) / 2;

const calloutBlock = BlockType(
  kind: Kinds.callout,
  label: 'Callout',
  hint: 'Make writing stand out.',
  iconX: Ix.callout,
  aliases: 'callout note info tip',
  placeholder: 'Type something…',
  decorate: _decorate,
  handleTop: _handleTop,
  paintsColor: true,
);
