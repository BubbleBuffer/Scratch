// More app icons for the editor and page chrome (toggle, callout, image,
// history...): Lucide glyphs like icons.dart. [IconBtnX] is [IconBtn] for
// these icons.

import 'package:flutter/widgets.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import 'hover.dart';
import 'lucide.dart';
import 'tokens.dart';
import 'tooltip.dart';

enum Ix {
  triangle,
  callout,
  columns,
  image,
  file,
  bookmark,
  toc,
  mention,
  calendar,
  color,
  underline,
  bold,
  italic,
  strike,
  redo,
  restore,
  lock,
  history,
  import,
  export,
  starFilled,
  smile,
  fullWidth,
  textSize,
  move,
  linkPage,
  subPage,
  collapse,
  menu,
  backlink,
  select,
  folder,
  reposition,
  arrowRight,
  table,

  /// Send feedback (Lucide message-square-warning).
  feedback,
}

/// The Lucide glyph behind each [Ix].
const ixGlyphs = <Ix, LucideGlyph>{
  Ix.triangle: (LucideIcons.play300, LucideIcons.play, LucideIcons.play500), // play
  Ix.callout: (LucideIcons.squareDot300, LucideIcons.squareDot, LucideIcons.squareDot500), // square-dot
  Ix.columns: (LucideIcons.columns2300, LucideIcons.columns2, LucideIcons.columns2500), // columns-2
  Ix.image: (LucideIcons.image300, LucideIcons.image, LucideIcons.image500), // image
  Ix.file: (LucideIcons.paperclip300, LucideIcons.paperclip, LucideIcons.paperclip500), // paperclip
  Ix.bookmark: (LucideIcons.bookmark300, LucideIcons.bookmark, LucideIcons.bookmark500), // bookmark
  Ix.toc: (LucideIcons.tableOfContents300, LucideIcons.tableOfContents, LucideIcons.tableOfContents500), // table-of-contents
  Ix.mention: (LucideIcons.atSign300, LucideIcons.atSign, LucideIcons.atSign500), // at-sign
  Ix.calendar: (LucideIcons.calendar300, LucideIcons.calendar, LucideIcons.calendar500), // calendar
  Ix.color: (LucideIcons.baseline300, LucideIcons.baseline, LucideIcons.baseline500), // baseline
  Ix.underline: (LucideIcons.underline300, LucideIcons.underline, LucideIcons.underline500), // underline
  Ix.bold: (LucideIcons.bold300, LucideIcons.bold, LucideIcons.bold500), // bold
  Ix.italic: (LucideIcons.italic300, LucideIcons.italic, LucideIcons.italic500), // italic
  Ix.strike: (LucideIcons.strikethrough300, LucideIcons.strikethrough, LucideIcons.strikethrough500), // strikethrough
  Ix.redo: (LucideIcons.redo2300, LucideIcons.redo2, LucideIcons.redo2500), // redo-2
  Ix.restore: (LucideIcons.rotateCcw300, LucideIcons.rotateCcw, LucideIcons.rotateCcw500), // rotate-ccw
  Ix.lock: (LucideIcons.lock300, LucideIcons.lock, LucideIcons.lock500), // lock
  Ix.history: (LucideIcons.history300, LucideIcons.history, LucideIcons.history500), // history
  Ix.import: (LucideIcons.download300, LucideIcons.download, LucideIcons.download500), // download
  Ix.export: (LucideIcons.upload300, LucideIcons.upload, LucideIcons.upload500), // upload
  Ix.starFilled: (LucideIcons.star300, LucideIcons.star, LucideIcons.star500), // star
  Ix.smile: (LucideIcons.smile300, LucideIcons.smile, LucideIcons.smile500), // smile
  Ix.fullWidth: (LucideIcons.moveHorizontal300, LucideIcons.moveHorizontal, LucideIcons.moveHorizontal500), // move-horizontal
  Ix.textSize: (LucideIcons.aLargeSmall300, LucideIcons.aLargeSmall, LucideIcons.aLargeSmall500), // a-large-small
  Ix.move: (LucideIcons.folderInput300, LucideIcons.folderInput, LucideIcons.folderInput500), // folder-input
  Ix.linkPage: (LucideIcons.fileSymlink300, LucideIcons.fileSymlink, LucideIcons.fileSymlink500), // file-symlink
  Ix.subPage: (LucideIcons.filePlus300, LucideIcons.filePlus, LucideIcons.filePlus500), // file-plus
  Ix.collapse: (LucideIcons.chevronsLeft300, LucideIcons.chevronsLeft, LucideIcons.chevronsLeft500), // chevrons-left
  Ix.menu: (LucideIcons.menu300, LucideIcons.menu, LucideIcons.menu500), // menu
  Ix.backlink: (LucideIcons.arrowUpLeft300, LucideIcons.arrowUpLeft, LucideIcons.arrowUpLeft500), // arrow-up-left
  Ix.select: (LucideIcons.squareCheckBig300, LucideIcons.squareCheckBig, LucideIcons.squareCheckBig500), // square-check-big
  Ix.folder: (LucideIcons.folder300, LucideIcons.folder, LucideIcons.folder500), // folder
  Ix.reposition: (LucideIcons.move300, LucideIcons.move, LucideIcons.move500), // move
  Ix.arrowRight: (LucideIcons.arrowRight300, LucideIcons.arrowRight, LucideIcons.arrowRight500), // arrow-right
  Ix.table: (LucideIcons.table300, LucideIcons.table, LucideIcons.table500), // table
  Ix.feedback: (LucideIcons.messageSquareWarning300, LucideIcons.messageSquareWarning, LucideIcons.messageSquareWarning500), // message-square-warning
};

/// Filled variants: the outline's inside, painted under the glyph.
const _ixFill = <Ix, List<Offset>>{
  Ix.triangle: lucidePlayFill,
  Ix.starFilled: lucideStarFill,
};

class IconX extends StatelessWidget {
  const IconX(this.icon, {super.key, this.size = 20, required this.color, this.stroke = 1.5});
  final Ix icon;
  final double size;
  final Color color;

  /// Line weight in px; picks the nearest Lucide weight.
  final double stroke;

  @override
  Widget build(BuildContext context) => LucideIcon(lucideWeight(ixGlyphs[icon]!, size, stroke),
      size: size, color: color, fill: _ixFill[icon]);
}

/// A square button with an [Ix] icon (the counterpart of IconBtn).
class IconBtnX extends StatelessWidget {
  const IconBtnX(this.icon,
      {super.key,
      this.onTap,
      this.size = 28,
      this.glyph = 20,
      this.color,
      this.radius = Radii.md,
      this.selected = false,
      this.semanticLabel,
      this.tooltip,
      this.command});
  final Ix icon;
  final VoidCallback? onTap;
  final double size, glyph, radius;
  final Color? color;
  final bool selected;
  final String? semanticLabel;

  /// Tooltip text; defaults to [semanticLabel] (see IconBtn).
  final String? tooltip;

  /// The registered command this button runs; the tooltip shows its shortcut.
  final String? command;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final tip = tooltip ?? semanticLabel;
    final button = Semantics(
      button: true,
      label: semanticLabel ?? tooltip ?? icon.name,
      child: Hover(
        onTap: onTap,
        radius: radius,
        selected: selected,
        selectedColor: c.bgPressed,
        child: SizedBox(
          width: size,
          height: size,
          child: Center(child: IconX(icon, size: glyph, color: color ?? c.iconSecondary)),
        ),
      ),
    );
    return tip == null ? button : AppTooltip(label: tip, command: command, child: button);
  }
}
