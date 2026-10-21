// Numbered list: 1. / a. / i. by depth; numbering counts a run of numbered
// siblings (Doc.reflatten sets Node.number).

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../registry/block_registry.dart';
import '../ui/ui.dart';
import 'list_marker.dart';

String _alpha(int n) {
  var s = '';
  while (n > 0) {
    n--;
    s = String.fromCharCode(97 + n % 26) + s;
    n ~/= 26;
  }
  return s;
}

String _roman(int n) {
  const v = [1000, 900, 500, 400, 100, 90, 50, 40, 10, 9, 5, 4, 1];
  const r = ['m', 'cm', 'd', 'cd', 'c', 'xc', 'l', 'xl', 'x', 'ix', 'v', 'iv', 'i'];
  var s = '';
  for (var i = 0; i < v.length; i++) {
    while (n >= v[i]) {
      s += r[i];
      n -= v[i];
    }
  }
  return s;
}

String numberLabel(int number, int depth) => switch (depth % 3) {
      0 => '$number.',
      1 => '${_alpha(number)}.',
      _ => '${_roman(number)}.',
    };

Widget _decorate(BlockContext b, Widget text) => markerRow(
      b,
      Align(
        alignment: Alignment.centerRight,
        child: Padding(
          padding: const EdgeInsets.only(right: 2),
          child: Text(numberLabel(b.node.number, b.node.depth), style: b.style.copyWith(decoration: TextDecoration.none)),
        ),
      ),
      text,
    );

const numberedBlock = BlockType(
  kind: Kinds.numbered,
  label: 'Numbered list',
  hint: 'Create a list with numbering.',
  icon: Ico.numbers,
  aliases: 'numbered list ol ordered',
  shortcut: '1.',
  markdown: ['1. '],
  listItem: true,
  exitOnEmptyEnter: true,
  placeholder: 'List',
  decorate: _decorate,
);
