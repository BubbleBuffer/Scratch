// Shared layout of list items: a 24 px marker column before the text.

import 'package:flutter/widgets.dart';

import '../registry/block_registry.dart';

Widget markerRow(BlockContext b, Widget marker, Widget text) => Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(width: 24, height: b.lineHeight, child: marker),
        const SizedBox(width: 2),
        Expanded(child: text),
      ],
    );
