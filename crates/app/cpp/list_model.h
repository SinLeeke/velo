// Generic list model for the Velo.Backend singletons.
//
// Rows are flat records whose fields are the role names given at construction. Rust fills it
// through the C functions at the bottom (JSON in, always called on the Qt thread). It is a plain
// QAbstractListModel, so ListView/Repeater reuse delegates; `count` and `get(i)` are provided
// because the preview mocks (ListModel) expose them.

#pragma once

#include <QtCore/QAbstractListModel>
#include <QtCore/QStringList>
#include <QtCore/QVariantList>
#include <QtCore/QVariantMap>
#include <QtCore/QVector>

class VeloListModel : public QAbstractListModel
{
    Q_OBJECT
    Q_PROPERTY(int count READ count NOTIFY countChanged)

public:
    explicit VeloListModel(const QStringList &roles, QObject *parent = nullptr);

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    int count() const { return m_rows.size(); }
    Q_INVOKABLE QVariantMap get(int row) const;

    void replace(const QVariantList &rows);
    void append(const QVariantList &rows);
    void insertAt(int row, const QVariantList &rows);
    void setField(int row, const QString &role, const QVariant &value);
    void removeAt(int row);
    void moveRow(int from, int to);

signals:
    void countChanged();

private:
    QVector<QVariant> toRow(const QVariant &object) const;

    QStringList m_roles;
    QVector<QVector<QVariant>> m_rows;
};

extern "C" {
void *velo_model_new(const char *roles_csv, void *parent);
void velo_model_replace(void *model, const char *json, int len);
void velo_model_append(void *model, const char *json, int len);
void velo_model_insert(void *model, int row, const char *json, int len);
void velo_model_set_field(void *model, int row, const char *role, const char *json, int len);
void velo_model_remove_row(void *model, int row);
void velo_model_move_row(void *model, int from, int to);
int velo_model_count(void *model);
}
