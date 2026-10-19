# MoQCast 品牌图标

沿用应用现有 `mac/assets/icons/MoQCast.iconset/icon_128x128.png`，原始 PNG 不改色、不裁切。`moqcast.rgba` 是同一128×128图像解码后的RGBA8字节，供 shared-ui 直接上传纹理，不新增运行时图像解码依赖。浅深主题均保留原图颜色。

重建：`Image.open("moqcast.png").convert("RGBA").tobytes()`（Pillow），输出65536字节。UI以32逻辑点显示；设备/来源图标仍使用各自语义图形。
