#include "list_model.h"

#include <QtCore/QByteArray>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>

VeloListModel::VeloListModel(const QStringList &roles, QObject *parent)
    : QAbstractListModel(parent), m_roles(roles)
{
}

int VeloListModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : m_rows.size();
}

QVariant VeloListModel::data(const QModelIndex &index, int role) const
{
    const int column = role - Qt::UserRole - 1;
    if (!index.isValid() || index.row() < 0 || index.row() >= m_rows.size() || column < 0
        || column >= m_roles.size())
        return {};
    return m_rows.at(index.row()).at(column);
}

QHash<int, QByteArray> VeloListModel::roleNames() const
{
    QHash<int, QByteArray> names;
    for (int i = 0; i < m_roles.size(); ++i)
        names.insert(Qt::UserRole + 1 + i, m_roles.at(i).toUtf8());
    return names;
}

QVariantMap VeloListModel::get(int row) const
{
    QVariantMap map;
    if (row < 0 || row >= m_rows.size())
        return map;
    for (int i = 0; i < m_roles.size(); ++i)
        map.insert(m_roles.at(i), m_rows.at(row).at(i));
    return map;
}

QVector<QVariant> VeloListModel::toRow(const QVariant &object) const
{
    const QVariantMap map = object.toMap();
    QVector<QVariant> row;
    row.reserve(m_roles.size());
    for (const QString &role : m_roles)
        row.append(map.value(role));
    return row;
}

void VeloListModel::replace(const QVariantList &rows)
{
    const int before = m_rows.size();
    beginResetModel();
    m_rows.clear();
    m_rows.reserve(rows.size());
    for (const QVariant &object : rows)
        m_rows.append(toRow(object));
    endResetModel();
    if (before != m_rows.size())
        emit countChanged();
}

void VeloListModel::append(const QVariantList &rows)
{
    if (rows.isEmpty())
        return;
    const int first = m_rows.size();
    beginInsertRows(QModelIndex(), first, first + rows.size() - 1);
    for (const QVariant &object : rows)
        m_rows.append(toRow(object));
    endInsertRows();
    emit countChanged();
}

void VeloListModel::insertAt(int row, const QVariantList &rows)
{
    if (rows.isEmpty())
        return;
    const int at = qBound(0, row, int(m_rows.size()));
    beginInsertRows(QModelIndex(), at, at + rows.size() - 1);
    for (int i = 0; i < rows.size(); ++i)
        m_rows.insert(at + i, toRow(rows.at(i)));
    endInsertRows();
    emit countChanged();
}

void VeloListModel::setField(int row, const QString &role, const QVariant &value)
{
    const int column = m_roles.indexOf(role);
    if (row < 0 || row >= m_rows.size() || column < 0)
        return;
    if (m_rows[row][column] == value)
        return;
    m_rows[row][column] = value;
    const QModelIndex idx = index(row);
    emit dataChanged(idx, idx, { Qt::UserRole + 1 + column });
}

void VeloListModel::removeAt(int row)
{
    if (row < 0 || row >= m_rows.size())
        return;
    beginRemoveRows(QModelIndex(), row, row);
    m_rows.removeAt(row);
    endRemoveRows();
    emit countChanged();
}

void VeloListModel::moveRow(int from, int to)
{
    if (from < 0 || to < 0 || from >= m_rows.size() || to >= m_rows.size() || from == to)
        return;
    // `to` is the row's final index; Qt wants the destination as "insert before" in the
    // original numbering, one past `to` when moving down.
    beginMoveRows(QModelIndex(), from, from, QModelIndex(), to > from ? to + 1 : to);
    m_rows.move(from, to);
    endMoveRows();
}

namespace {
VeloListModel *model(void *p) { return static_cast<VeloListModel *>(p); }
QJsonDocument parse(const char *json, int len)
{
    return QJsonDocument::fromJson(QByteArray(json, len));
}
}

extern "C" {

void *velo_model_new(const char *roles_csv, void *parent)
{
    const QStringList roles = QString::fromUtf8(roles_csv).split(QLatin1Char(','), Qt::SkipEmptyParts);
    return new VeloListModel(roles, static_cast<QObject *>(parent));
}

void velo_model_replace(void *m, const char *json, int len)
{
    model(m)->replace(parse(json, len).array().toVariantList());
}

void velo_model_append(void *m, const char *json, int len)
{
    model(m)->append(parse(json, len).array().toVariantList());
}

void velo_model_insert(void *m, int row, const char *json, int len)
{
    model(m)->insertAt(row, parse(json, len).array().toVariantList());
}

void velo_model_set_field(void *m, int row, const char *role, const char *json, int len)
{
    // The value is wrapped in an array so that scalars parse as JSON documents.
    const QVariantList wrapped = parse(json, len).array().toVariantList();
    if (wrapped.isEmpty())
        return;
    model(m)->setField(row, QString::fromUtf8(role), wrapped.first());
}

void velo_model_remove_row(void *m, int row) { model(m)->removeAt(row); }

void velo_model_move_row(void *m, int from, int to) { model(m)->moveRow(from, to); }

int velo_model_count(void *m) { return model(m)->count(); }
}
