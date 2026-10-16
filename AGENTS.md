# Murasame 开发者 / AI Agent 开发规范

## 1. Slint UI 图标使用规范

### 1.1 严禁使用 Emoji / 文本符号代替 SVG 图标
- **严禁**偷懒使用 Unicode 字符或 Emoji（如 `">"`, `"✓"`, `"▲"`, `"▼"`, `"★"` 等）作为界面图标。
- **规范**：所有图标必须统一使用 `assets/icons/` 目录下的 `.svg` 矢量图标，通过 `@image-url("../assets/icons/xxx.svg")` 引入。若缺少相应图标，需创建轻量规范的 SVG 文件后引入。

---

## 2. Slint 图标对齐与居中规范

Slint 的布局引擎对图片与文本的默认对齐行为有特殊机制，请严格遵循以下模式以避免每次添加图标都出现“对不齐”的问题：

### 2.1 规则一：Image 没有 `vertical-alignment` 属性
- 在 Slint 中，`vertical-alignment` 仅对 `Text` 元素有效，对 `Image` 元素**无效**。
- 如果直接将 `Image` 和 `Text` 放在 `HorizontalLayout` 中，`Image` 会默认贴紧顶部拉伸或靠顶，而 `Text` 设置了 `vertical-alignment: center`，两者高度中线不一致，必定产生错位。

### 2.2 图标 + 文本（水平排列）的标准规范
图标必须外层包裹一个 `VerticalLayout { alignment: center; ... }`，并显式指定 `Image` 的 `width` 与 `height`：
```slint
HorizontalLayout {
    alignment: center; // 或 start
    spacing: 8px;

    // 图标容器：通过 VerticalLayout 纵向居中
    VerticalLayout {
        alignment: center;
        Image {
            width: 14px;
            height: 14px;
            source: @image-url("../assets/icons/example.svg");
        }
    }

    // 文本元素：显式垂直居中
    Text {
        text: "标题内容";
        color: #ffffff;
        font-size: 13px;
        vertical-alignment: center;
    }
}
```

### 2.3 单图标按钮 / 容器的标准居中规范
在固定宽高容器（如 `Rectangle`）中放置单个图标时，切勿直接放置，否则默认落在 `(0, 0)`。必须使用相对坐标计算居中：
```slint
Rectangle {
    width: 28px;
    height: 28px;
    border-radius: 6px;

    Image {
        x: (parent.width - self.width) / 2;
        y: (parent.height - self.height) / 2;
        width: 14px;
        height: 14px;
        source: @image-url("../assets/icons/example.svg");
    }
}
```

---

## 3. Slint 悬浮层与布局高度防拉伸规范
1. **弹出浮层 / 下拉菜单**：浮动悬浮卡片（如 `Rectangle`）必须显式设定 `height`（例如 `height: layout.preferred-height;`），不可省略 `height`，否则在 Slint 中会默认拉伸至父级高度（出现无限向下延伸的 bug）。
2. **表单卡片内布局**：`ScrollView` 内的 `VerticalLayout` 以及卡片内部的 `VerticalLayout` 应指定 `alignment: start;`，避免容器高度增大时子项之间的间距被无限等比拉大。

---

## 4. 单元测试编写规范

### 4.1 严禁编写破坏性或无意义的垃圾测试
**严禁编写废话测试**：不应为了凑测试数而编写只测试简单字符串加个百分号或只循环打印日志的测试，保持测试套件干净且有实际价值。
