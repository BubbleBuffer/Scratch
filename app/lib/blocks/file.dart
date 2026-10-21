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
              child: SizedBox(
                height: 28,
                child: Row(children: [
                  const SizedBox(width: 2),
                  IconX(Ix.file, size: 18, color: c.iconSecondary),
                  const SizedBox(width: 6),
                  Flexible(
                    child: Text(name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: tsRaw(16, 20.8, 400, missing ? c.textTertiary : c.textPrimary)),
                  ),
                  if (size.isNotEmpty) ...[
                    const SizedBox(width: 6),
                    Text(size, style: tsRaw(12, 16, 400, c.textTertiary)),
                  ],
                  if (missing) ...[
                    const SizedBox(width: 6),
                    Text('not on this device', style: tsRaw(12, 16, 400, c.textTertiary)),
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

double _top(BlockContext b) => (b.mobile ? 3 : 2) + 2;
