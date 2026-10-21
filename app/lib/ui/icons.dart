// The app's icons: Lucide glyphs (lucide.dart) behind the app's own names.
// New UI uses these enums (Ico here, Ix in icon_x.dart), never another set.

import 'package:flutter/widgets.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import 'lucide.dart';

enum Ico {
  plus,
  grip,
  chevronLeft,
  chevronRight,
  chevronDown,
  search,
  page,
  compose,
  home,
  inbox,
  more,
  trash,
  arrowUp,
  arrowDown,
  turnInto,
  duplicate,
  link,
  indent,
  outdent,
  keyboardHide,
  undo,
  bullets,
  numbers,
  todo,
  quote,
  code,
  divider,
  text,
  check,
  comment,
  star,
  clock,
  settings,
  sidebar,
  close,
}

/// The Lucide glyph behind each [Ico].
const icoGlyphs = <Ico, LucideGlyph>{
  Ico.plus: (LucideIcons.plus300, LucideIcons.plus, LucideIcons.plus500), // plus
  Ico.grip: (LucideIcons.gripVertical300, LucideIcons.gripVertical, LucideIcons.gripVertical500), // grip-vertical
  Ico.chevronLeft: (LucideIcons.chevronLeft300, LucideIcons.chevronLeft, LucideIcons.chevronLeft500), // chevron-left
  Ico.chevronRight: (LucideIcons.chevronRight300, LucideIcons.chevronRight, LucideIcons.chevronRight500), // chevron-right
  Ico.chevronDown: (LucideIcons.chevronDown300, LucideIcons.chevronDown, LucideIcons.chevronDown500), // chevron-down
  Ico.search: (LucideIcons.search300, LucideIcons.search, LucideIcons.search500), // search
  Ico.page: (LucideIcons.fileText300, LucideIcons.fileText, LucideIcons.fileText500), // file-text
  Ico.compose: (LucideIcons.squarePen300, LucideIcons.squarePen, LucideIcons.squarePen500), // square-pen
  Ico.home: (LucideIcons.house300, LucideIcons.house, LucideIcons.house500), // house
  Ico.inbox: (LucideIcons.inbox300, LucideIcons.inbox, LucideIcons.inbox500), // inbox
  Ico.more: (LucideIcons.ellipsis300, LucideIcons.ellipsis, LucideIcons.ellipsis500), // ellipsis
  Ico.trash: (LucideIcons.trash2300, LucideIcons.trash2, LucideIcons.trash2500), // trash-2
  Ico.arrowUp: (LucideIcons.arrowUp300, LucideIcons.arrowUp, LucideIcons.arrowUp500), // arrow-up
  Ico.arrowDown: (LucideIcons.arrowDown300, LucideIcons.arrowDown, LucideIcons.arrowDown500), // arrow-down
  Ico.turnInto: (LucideIcons.refreshCw300, LucideIcons.refreshCw, LucideIcons.refreshCw500), // refresh-cw
  Ico.duplicate: (LucideIcons.copy300, LucideIcons.copy, LucideIcons.copy500), // copy
  Ico.link: (LucideIcons.link300, LucideIcons.link, LucideIcons.link500), // link
  Ico.indent: (LucideIcons.listIndentIncrease300, LucideIcons.listIndentIncrease, LucideIcons.listIndentIncrease500), // list-indent-increase
  Ico.outdent: (LucideIcons.listIndentDecrease300, LucideIcons.listIndentDecrease, LucideIcons.listIndentDecrease500), // list-indent-decrease
  Ico.keyboardHide: (LucideIcons.keyboardOff300, LucideIcons.keyboardOff, LucideIcons.keyboardOff500), // keyboard-off
  Ico.undo: (LucideIcons.undo2300, LucideIcons.undo2, LucideIcons.undo2500), // undo-2
  Ico.bullets: (LucideIcons.list300, LucideIcons.list, LucideIcons.list500), // list
  Ico.numbers: (LucideIcons.listOrdered300, LucideIcons.listOrdered, LucideIcons.listOrdered500), // list-ordered
  Ico.todo: (LucideIcons.squareCheck300, LucideIcons.squareCheck, LucideIcons.squareCheck500), // square-check
  Ico.quote: (LucideIcons.textQuote300, LucideIcons.textQuote, LucideIcons.textQuote500), // text-quote
  Ico.code: (LucideIcons.code300, LucideIcons.code, LucideIcons.code500), // code
  Ico.divider: (LucideIcons.separatorHorizontal300, LucideIcons.separatorHorizontal, LucideIcons.separatorHorizontal500), // separator-horizontal
  Ico.text: (LucideIcons.type300, LucideIcons.type, LucideIcons.type500), // type
  Ico.check: (LucideIcons.check300, LucideIcons.check, LucideIcons.check500), // check
  Ico.comment: (LucideIcons.messageSquare300, LucideIcons.messageSquare, LucideIcons.messageSquare500), // message-square
  Ico.star: (LucideIcons.star300, LucideIcons.star, LucideIcons.star500), // star
  Ico.clock: (LucideIcons.clock300, LucideIcons.clock, LucideIcons.clock500), // clock
  Ico.settings: (LucideIcons.settings300, LucideIcons.settings, LucideIcons.settings500), // settings
  Ico.sidebar: (LucideIcons.panelLeft300, LucideIcons.panelLeft, LucideIcons.panelLeft500), // panel-left
  Ico.close: (LucideIcons.x300, LucideIcons.x, LucideIcons.x500), // x
};

class Icon20 extends StatelessWidget {
  const Icon20(this.icon, {super.key, this.size = 20, required this.color, this.stroke = 1.5});
  final Ico icon;
  final double size;
  final Color color;

  /// Line weight in px; picks the nearest Lucide weight.
  final double stroke;

  @override
  Widget build(BuildContext context) =>
      LucideIcon(lucideWeight(icoGlyphs[icon]!, size, stroke), size: size, color: color);
}

/// Paints [icon] into [rect] (square) on [canvas], as [Icon20] does.
void paintIco(Canvas canvas, Ico icon, Rect rect, Color color, {double stroke = 1.5}) =>
    paintLucide(canvas, lucideWeight(icoGlyphs[icon]!, rect.height, stroke), rect, color);
