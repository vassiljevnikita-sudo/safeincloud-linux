"""Compact, accessible native list rows for the selected Adwaita layout."""
from PySide6.QtCore import Qt, QSize, QRect, QRectF
from PySide6.QtGui import QColor, QFont, QPainter
from PySide6.QtWidgets import QStyledItemDelegate, QStyle, QCheckBox
from .qt_theme import icon
from .qt_system_theme import theme_colors

TITLE = Qt.UserRole + 1
LOGIN = Qt.UserRole + 2
FAVORITE = Qt.UserRole + 3
COUNT = Qt.UserRole + 4


class FavoriteSwitch(QCheckBox):
    """A keyboard-accessible checkbox painted as the chosen Adwaita switch."""
    def sizeHint(self):
        return QSize(46, 28)

    def hitButton(self, point):
        return self.rect().contains(point)

    def paintEvent(self, event):
        t = theme_colors()
        painter = QPainter(self)
        painter.setRenderHint(QPainter.Antialiasing)
        painter.setPen(Qt.NoPen)
        painter.setBrush(QColor(t['primary'] if self.isChecked() else t['switch_off']))
        painter.setOpacity(1 if self.isEnabled() else .5)
        painter.drawRoundedRect(QRectF(1, 2, 44, 24), 12, 12)
        painter.setBrush(QColor(t['primary_text']))
        painter.drawEllipse(QRectF(24 if self.isChecked() else 4, 5, 18, 18))
        if self.hasFocus():
            painter.setBrush(Qt.NoBrush)
            painter.setPen(QColor(t['accent']))
            painter.drawRoundedRect(QRectF(0, 1, 46, 26), 13, 13)


class EntryDelegate(QStyledItemDelegate):
    def sizeHint(self, option, index):
        return QSize(280, 60)

    def paint(self, painter, option, index):
        t = theme_colors()
        painter.save()
        painter.setRenderHint(QPainter.Antialiasing)
        rect = option.rect.adjusted(6, 2, -6, -2)
        selected = bool(option.state & QStyle.State_Selected)
        if selected or option.state & QStyle.State_MouseOver:
            painter.setPen(Qt.NoPen)
            painter.setBrush(QColor(t['entry_selected'] if selected else t['entry_hover']))
            painter.drawRoundedRect(rect, 8, 8)
        if option.state & QStyle.State_HasFocus:
            painter.setBrush(Qt.NoBrush)
            painter.setPen(QColor(t['accent']))
            painter.drawRoundedRect(rect.adjusted(1, 1, -1, -1), 8, 8)
        title = index.data(TITLE) or ''
        colors = t['avatars']
        bg, fg = colors[sum(ord(c) for c in title) % len(colors)]
        circle = QRect(rect.x() + 10, rect.y() + 10, 36, 36)
        painter.setPen(Qt.NoPen)
        painter.setBrush(QColor(bg))
        painter.drawEllipse(circle)
        font = QFont('Atkinson Hyperlegible Next', 12)
        font.setWeight(QFont.DemiBold)
        painter.setFont(font)
        painter.setPen(QColor(fg))
        painter.drawText(circle, Qt.AlignCenter, title[:1].upper())
        x = circle.right() + 13
        width = max(10, rect.right() - x - 29)
        font.setPixelSize(15)
        font.setWeight(QFont.DemiBold if selected else QFont.Medium)
        painter.setFont(font)
        painter.setPen(QColor(t['ink']))
        painter.drawText(QRect(x, rect.y() + 7, width, 22), Qt.AlignVCenter,
                         painter.fontMetrics().elidedText(title, Qt.ElideRight, width))
        font.setPixelSize(13)
        font.setWeight(QFont.Normal)
        painter.setFont(font)
        painter.setPen(QColor(t['muted']))
        painter.drawText(QRect(x, rect.y() + 29, width, 19), Qt.AlignVCenter,
                         painter.fontMetrics().elidedText(index.data(LOGIN) or 'Passwort-Eintrag', Qt.ElideRight, width))
        if index.data(FAVORITE):
            icon('star').paint(painter, QRect(rect.right() - 25, rect.y() + 19, 16, 16))
        painter.restore()


class NavigationDelegate(QStyledItemDelegate):
    def sizeHint(self, option, index):
        return QSize(190, 76 if index.row() == 2 else 40)

    def paint(self, painter, option, index):
        t = theme_colors()
        painter.save()
        painter.setRenderHint(QPainter.Antialiasing)
        rect = option.rect.adjusted(6, 2, -6, -2)
        font = QFont('Atkinson Hyperlegible Next')
        font.setPixelSize(15)
        if index.row() == 2:
            painter.setFont(font)
            painter.setPen(QColor(t['muted']))
            painter.drawText(rect.adjusted(12, 6, 0, 0), Qt.AlignTop, 'Labels')
            rect.setTop(rect.top() + 36)
        selected = bool(option.state & QStyle.State_Selected)
        if selected or option.state & QStyle.State_MouseOver:
            painter.setPen(Qt.NoPen)
            painter.setBrush(QColor(t['nav_selected'] if selected else t['nav_hover']))
            painter.drawRoundedRect(rect, 7, 7)
        if option.state & QStyle.State_HasFocus:
            painter.setBrush(Qt.NoBrush)
            painter.setPen(QColor(t['accent']))
            painter.drawRoundedRect(rect.adjusted(1, 1, -1, -1), 7, 7)
        icon('inbox' if index.row() == 0 else 'star' if index.row() == 1 else 'tag').paint(
            painter, QRect(rect.x() + 12, rect.center().y() - 8, 17, 17))
        font.setWeight(QFont.DemiBold if selected else QFont.Normal)
        painter.setFont(font)
        painter.setPen(QColor(t['ink']))
        text = index.data(Qt.DisplayRole) or ''
        painter.drawText(rect.adjusted(42, 0, -28, 0), Qt.AlignVCenter,
                         painter.fontMetrics().elidedText(text, Qt.ElideRight, rect.width() - 72))
        font.setPixelSize(13)
        font.setWeight(QFont.Normal)
        painter.setFont(font)
        painter.setPen(QColor(t['muted']))
        painter.drawText(rect.adjusted(0, 0, -12, 0), Qt.AlignRight | Qt.AlignVCenter, str(index.data(COUNT) or 0))
        painter.restore()
