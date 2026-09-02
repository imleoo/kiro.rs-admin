import { useEffect, useState } from 'react'
import { Blocks, Plus, Trash2, Save } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
} from '@/components/ui/dialog'
import { useCustomModels, useSetCustomModels } from '@/hooks/use-custom-models'
import { extractErrorMessage } from '@/lib/utils'
import type { CustomModel } from '@/types/api'

interface CustomModelsDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

function emptyModel(): CustomModel {
  return {
    id: '',
    backendId: '',
    displayName: null,
    contextWindow: null,
    maxTokens: null,
    supportsReasoning: null,
    ownedBy: null,
  }
}

/**
 * 自定义模型管理：新增或覆盖后端模型定义（会出现在 /v1/models 里，可覆盖内置模型
 * 的元数据）。与"模型映射"（topbar 另一个入口）是两套不同的能力，互不影响：
 * 模型映射只做请求时的名字转发，源名不出现在 /v1/models。
 *
 * 整表替换保存：本地编辑草稿，点「保存」一次性提交全部，运行期无需重启即可生效。
 */
export function CustomModelsDialog({ open, onOpenChange }: CustomModelsDialogProps) {
  const { data, isLoading } = useCustomModels()
  const { mutate: save, isPending: saving } = useSetCustomModels()

  const [draft, setDraft] = useState<CustomModel[]>([])

  useEffect(() => {
    if (open && data) {
      setDraft(data.customModels)
    }
  }, [open, data])

  const updateRow = (index: number, patch: Partial<CustomModel>) => {
    setDraft((rows) => rows.map((row, i) => (i === index ? { ...row, ...patch } : row)))
  }

  const removeRow = (index: number) => {
    setDraft((rows) => rows.filter((_, i) => i !== index))
  }

  const addRow = () => {
    setDraft((rows) => [...rows, emptyModel()])
  }

  const handleSave = () => {
    const rows = draft.filter((row) => row.id.trim() && row.backendId.trim())
    if (rows.length !== draft.length) {
      toast.error('存在未填写 id 或 backendId 的行，已跳过保存这些行')
    }
    save(rows, {
      onSuccess: () => toast.success('自定义模型已保存并热生效'),
      onError: (err) => toast.error(`保存失败: ${extractErrorMessage(err)}`),
    })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Blocks className="h-4 w-4" />
            自定义模型
          </DialogTitle>
          <DialogDescription>
            新增或覆盖一个完整的后端模型定义，会出现在 <code>/v1/models</code> 里；
            <code>id</code> 是客户端请求用的别名（大小写不敏感），
            <code>backendId</code> 是实际下发给 Kiro 上游的模型 ID。保存后立即热生效，
            无需重启。
          </DialogDescription>
        </DialogHeader>

        <div className="max-h-[26rem] space-y-3 overflow-y-auto py-1">
          {isLoading ? (
            <p className="py-6 text-center text-sm text-muted-foreground">加载中…</p>
          ) : draft.length === 0 ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              暂无自定义模型，点下方「添加」新建一条。
            </p>
          ) : (
            draft.map((row, index) => (
              <div
                key={index}
                className="space-y-2 rounded-md border border-border/60 bg-secondary/30 p-3"
              >
                <div className="flex items-center gap-2">
                  <label className="flex-1 text-xs font-medium text-muted-foreground">
                    id（别名）
                    <Input
                      placeholder="my-opus"
                      value={row.id}
                      onChange={(e) => updateRow(index, { id: e.target.value })}
                      disabled={saving}
                      className="mt-1 h-8 font-mono text-[13px]"
                    />
                  </label>
                  <label className="flex-1 text-xs font-medium text-muted-foreground">
                    backendId（Kiro 后端模型 ID）
                    <Input
                      placeholder="claude-opus-4.8"
                      value={row.backendId}
                      onChange={(e) => updateRow(index, { backendId: e.target.value })}
                      disabled={saving}
                      className="mt-1 h-8 font-mono text-[13px]"
                    />
                  </label>
                  <Button
                    type="button"
                    size="icon"
                    variant="ghost"
                    className="mt-4 h-8 w-8 shrink-0 text-muted-foreground hover:text-destructive"
                    onClick={() => removeRow(index)}
                    disabled={saving}
                    title="删除"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </div>

                <div className="flex flex-wrap items-end gap-2">
                  <label className="w-40 text-xs font-medium text-muted-foreground">
                    展示名（可选）
                    <Input
                      placeholder="缺省用 id"
                      value={row.displayName ?? ''}
                      onChange={(e) =>
                        updateRow(index, { displayName: e.target.value || null })
                      }
                      disabled={saving}
                      className="mt-1 h-8 text-[13px]"
                    />
                  </label>
                  <label className="w-32 text-xs font-medium text-muted-foreground">
                    上下文窗口
                    <Input
                      type="number"
                      placeholder="200000"
                      value={row.contextWindow ?? ''}
                      onChange={(e) =>
                        updateRow(index, {
                          contextWindow: e.target.value ? Number(e.target.value) : null,
                        })
                      }
                      disabled={saving}
                      className="mt-1 h-8 text-[13px]"
                    />
                  </label>
                  <label className="w-32 text-xs font-medium text-muted-foreground">
                    最大输出 token
                    <Input
                      type="number"
                      placeholder="64000"
                      value={row.maxTokens ?? ''}
                      onChange={(e) =>
                        updateRow(index, {
                          maxTokens: e.target.value ? Number(e.target.value) : null,
                        })
                      }
                      disabled={saving}
                      className="mt-1 h-8 text-[13px]"
                    />
                  </label>
                  <label className="w-28 text-xs font-medium text-muted-foreground">
                    owned_by
                    <Input
                      placeholder="custom"
                      value={row.ownedBy ?? ''}
                      onChange={(e) => updateRow(index, { ownedBy: e.target.value || null })}
                      disabled={saving}
                      className="mt-1 h-8 text-[13px]"
                    />
                  </label>
                  <label className="flex h-8 items-center gap-1.5 text-xs font-medium text-muted-foreground">
                    <Checkbox
                      checked={row.supportsReasoning ?? false}
                      onCheckedChange={(checked) =>
                        updateRow(index, { supportsReasoning: checked === true })
                      }
                      disabled={saving}
                    />
                    支持 reasoning
                  </label>
                </div>
              </div>
            ))
          )}
        </div>

        <div className="flex items-center justify-between pt-1">
          <Button type="button" variant="outline" size="sm" onClick={addRow} disabled={saving}>
            <Plus className="h-3.5 w-3.5" />
            添加
          </Button>
          <Button type="button" size="sm" onClick={handleSave} disabled={saving}>
            <Save className="h-3.5 w-3.5" />
            {saving ? '保存中…' : '保存'}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
