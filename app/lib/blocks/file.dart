// File: an attachment shown as a 28px row with a file icon, its name and
// size, plus an optional caption.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../registry/block_registry.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';
import 'attachment.dart';
import 'media.dart';

Widget _decorate(BlockContext b, Widget text) {
  final t = b.node.target;
  final content = t == null || t.isEmpty
      ? emptyMedia(b, Ix.file, 'Upload or embed a file')
      : AttachmentBuilder(
          key: ValueKey('file-${b.node.id}'),
          id: t,
          builder: (context, a, loading) {
            final c = context.colors;
            final name = a?.info?.name ?? (loading ? '' : 'File');
            final size = a?.info == null ? '' : formatBytes(a!.info!.size.toInt());
            final missing = !loading && (a == null || a.path.isEmpty);
            return Hover(
              radius: Radii.sm,
              child: ConstrainedBox(
                constraints: BoxConstraints(minHeight: ControlSize.sm.height),
                child: Row(children: [
                  const Gap(Space.hair),
                  IconX(Ix.file, size: IconSize.md.px, color: c.iconSecondary),
                  const Gap(Space.xs),
                  Flexible(
                    child: Text(name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: ts(context.type.body, missing ? c.textTertiary : c.textPrimary)),
                  ),
                  if (size.isNotEmpty) ...[
                    const Gap(Space.xs),
                    Text(size, style: ts(context.type.uiSmall, c.textTertiary, weight: Weight.regular)),
                  ],
                  if (missing) ...[
                    const Gap(Space.xs),
                    Text('not on this device', style: ts(context.type.uiSmall, c.textTertiary, weight: Weight.regular)),
                  ],
                ]),
              ),
            );
          },
        );
  return withCaption(b, content, text);
}

const fileBlock = BlockType(
  kind: Kinds.file,
  label: 'File',
  hint: 'Upload or embed with a link.',
  iconX: Ix.file,
  aliases: 'file attachment upload pdf',
  placeholder: 'Add a caption…',
  textStyle: captionStyle,
  decorate: _decorate,
  handleTop: _top,
  atomic: true,
  acceptsChildren: false,
  colorable: false,
  group: 'Media',
);

double _top(BlockContext b) => BlockRhythm.pad(b.touch) + Space.hair;
