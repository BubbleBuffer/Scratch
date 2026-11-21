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
      Space.md,
      b.touch ? Space.md : Space.lg,
      Space.lg,
      b.touch ? Space.md : Space.lg,
    ),
    child: Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        // The emoji is centred on the first line of text.
        SizedBox(
          height: CalloutParts.icon,
          child: Builder(
            builder: (anchor) => Hover(
              key: ValueKey('callout-icon-${n.id}'),
              borderRadius: Radii.ctl,
              background: b.colors.bgPrimary,
              onTap: () => b.host.openEmojiPicker(n, anchor),
              child: SizedBox(
                width: CalloutParts.icon,
                height: CalloutParts.icon,
                child: Center(
                  child: Text(
                    icon,
                    style: emojiStyle(CalloutParts.emoji, emojiFallback),
                  ),
                ),
              ),
            ),
          ),
        ),
        const Gap(Space.sm),
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
        top: CalloutParts.barInset,
        bottom: CalloutParts.barInset,
        width: Stroke.seam,
        child: IgnorePointer(
          child: DecoratedBox(
            decoration: BoxDecoration(
              color: patch.fill,
              borderRadius: BorderRadius.circular(Radii.sm),
            ),
          ),
        ),
      ),
    ],
  );
}

double _handleTop(BlockContext b) =>
    BlockRhythm.pad(b.touch) +
    (b.touch ? Space.md : Space.lg) +
    (CalloutParts.icon - BlockRhythm.handleLine) / 2;

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
