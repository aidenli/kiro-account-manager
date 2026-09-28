import type React from 'react'
import { useState, useMemo, useEffect } from 'react'
import { RotateCw, TrendingUp, Shuffle, Zap, Users, CheckCircle2, Search, Ban, AlertCircle } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { Checkbox } from '@/components/ui/checkbox'
import { Badge } from '@/components/ui/badge'
import { formatUsage, getQuota, getUsed } from '@/utils/accountStats'

const POOL_STATUS_COLORS: Record<string, string> = {
  active: 'bg-green-500',
  overage: 'bg-purple-500',
  capped: 'bg-yellow-500',
  banned: 'bg-red-500',
  invalid: 'bg-gray-500',
}
const POOL_STATUS_ORDER: Record<string, number> = { active: 0, overage: 1, '': 0, capped: 2, invalid: 3, banned: 4 }
const POOL_STATUS_KEYS: Record<string, string> = {
  active: 'gateway.poolStatusActive',
  overage: 'gateway.poolStatusOverage',
  capped: 'gateway.poolStatusCapped',
  banned: 'gateway.poolStatusBanned',
  invalid: 'gateway.poolStatusInvalid',
}
import { Textarea } from '@/components/ui/textarea'
import { GatewaySurfaceCard } from './GatewayShared'
import { countEffectiveClientApiKeys, parseClientApiKeys } from './gatewayPageUtils'
import ModelMappingDialog from './ModelMappingDialog'
import ApiKeysDialog from './ApiKeysDialog'
import PromptFilterRulesDialog from './PromptFilterRulesDialog'
import {
  DialogRoot,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogBody,
  DialogFooter
} from '@/components/shared/dialog'
import { useApp } from '../../../hooks/useApp'

interface GatewayConfigProps {
  config: any;
  fieldErrors: Record<string, string>;
  setField: (key: string, value: any) => void;
  accountOptions: any[];
  groupOptions: any[];
  setConfig: React.Dispatch<React.SetStateAction<any>>;
  applyGatewayLocalOnlyChange: (config: any, checked: boolean, generator: () => string) => any;
  createGeneratedApiKey: () => string;
  handleSaveConfig: () => Promise<void>;
  handleAutoStartToggle: (checked: boolean) => Promise<void>;
  onShowClientConfig?: () => void;
  hasConfiguredClients?: boolean;
  /** 由父级（接入引导区）触发打开密钥管理弹窗 */
  openApiKeysDialog?: boolean;
  onApiKeysDialogOpened?: () => void;
}

function GatewayConfig({
  config,
  fieldErrors,
  setField,
  accountOptions,
  groupOptions,
  setConfig,
  applyGatewayLocalOnlyChange,
  createGeneratedApiKey,
  handleSaveConfig,
  handleAutoStartToggle,
  onShowClientConfig,
  hasConfiguredClients = false,
  openApiKeysDialog = false,
  onApiKeysDialogOpened,
}: GatewayConfigProps) {
  const { t } = useApp()
  const [showModelMappingDialog, setShowModelMappingDialog] = useState(false)
  const [showApiKeysDialog, setShowApiKeysDialog] = useState(false)

  // 父级（接入引导区）请求打开密钥弹窗：响应后立即回吐信号，避免重复触发
  useEffect(() => {
    if (!openApiKeysDialog) return
    setShowApiKeysDialog(true)
    onApiKeysDialogOpened?.()
  }, [openApiKeysDialog, onApiKeysDialogOpened])
  const [showPromptFilterRulesDialog, setShowPromptFilterRulesDialog] = useState(false)
  const [showAccountPoolDialog, setShowAccountPoolDialog] = useState(false)
  const [poolSearchQuery, setPoolSearchQuery] = useState('')
  const [poolStatusFilter, setPoolStatusFilter] = useState<string>('all')

  const getStrategyLabel = (strategy: string) => {
    const STRATEGY_KEYS: Record<string, string> = {
      round_robin: 'gateway.roundRobin',
      random: 'gateway.random',
      balanced: 'gateway.strategyBalanced',
      most_quota: 'gateway.strategyMostQuota',
      weighted_random: 'gateway.weightedRandom',
      least_connections: 'gateway.leastConnections',
    }
    return STRATEGY_KEYS[strategy] ? t(STRATEGY_KEYS[strategy]) : strategy
  }

  const selectedPoolAccountIds = Array.isArray(config.poolAccountIds) ? config.poolAccountIds : []
  // 配置里的 id 可能指向已删除的账号。分母是「现存可选账号」，故分子也只该数
  // 真正能解析到账号的 id，否则会出现「已选 23 / 共 0」这种分母小于分子的昏话。
  const availablePoolIdSet = new Set(accountOptions.map((o: any) => o.value))
  const effectiveSelectedPoolIds = selectedPoolAccountIds.filter((id: string) =>
    availablePoolIdSet.has(id)
  )
  const selectedPoolAccounts = accountOptions.filter((account: any) => selectedPoolAccountIds.includes(account.value))
  const effectiveStrategy = config.strategy || 'round_robin'

  const filteredPoolAccounts = useMemo(() => {
    return accountOptions
      .filter((opt: any) => {
        const acct = opt.account
        const status = acct?.status || 'active'
        if (poolStatusFilter !== 'all' && status !== poolStatusFilter) return false
        if (poolSearchQuery.trim()) {
          const q = poolSearchQuery.toLowerCase()
          const email = (acct?.email || '').toLowerCase()
          const userId = (acct?.userId || '').toLowerCase()
          const provider = (acct?.provider || '').toLowerCase()
          return email.includes(q) || userId.includes(q) || provider.includes(q)
        }
        return true
      })
      .sort((a: any, b: any) => {
        const sa = POOL_STATUS_ORDER[a.account?.status || ''] ?? 5
        const sb = POOL_STATUS_ORDER[b.account?.status || ''] ?? 5
        if (sa !== sb) return sa - sb
        const ca = selectedPoolAccountIds.includes(a.value) ? 0 : 1
        const cb = selectedPoolAccountIds.includes(b.value) ? 0 : 1
        return ca - cb
      })
  }, [accountOptions, poolSearchQuery, poolStatusFilter, selectedPoolAccountIds])

  const togglePoolAccount = (accountId: string) => {
    const next = selectedPoolAccountIds.includes(accountId)
      ? selectedPoolAccountIds.filter((id: string) => id !== accountId)
      : [...selectedPoolAccountIds, accountId]
    setField('poolAccountIds', next)
  }

  const toggleAllPoolAccounts = () => {
    // 用「现存账号是否都已在池中」判定，而非直接比较长度：
    // 池里可能残留已删除账号的 id，长度相等不代表选满
    if (accountOptions.length > 0 && effectiveSelectedPoolIds.length === accountOptions.length) {
      setField('poolAccountIds', [])
    } else {
      setField('poolAccountIds', accountOptions.map((account: any) => account.value))
    }
  }

  return (
    <div className="grid grid-cols-1 gap-3">
      <GatewaySurfaceCard>
        <div className="flex flex-col gap-3">
          <div className="flex flex-col gap-4">
            {/* Section 1: 网络与路由 */}
            <div className="space-y-3">
              <div className="text-sm font-medium text-foreground flex items-center gap-2">
                <div className="w-1 h-4 bg-primary rounded-full"></div>
                {t('gateway.sectionNetworkRouting')}
              </div>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                <div className="flex flex-col gap-1.5">
                  <Label>{t('gateway.listenAddress')}</Label>
                  <Input
                    value={config.host}
                    onChange={(e: React.ChangeEvent<HTMLInputElement>) => setField('host', e.target.value || '127.0.0.1')}
                    className={fieldErrors.host ? 'border-red-500' : ''}
                  />
                  {fieldErrors.host && <div className="text-xs text-red-500">{fieldErrors.host}</div>}
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label>{t('gateway.port')}</Label>
                  <Input
                    type="number"
                    value={config.port}
                    min={1}
                    max={65535}
                    onChange={(e: React.ChangeEvent<HTMLInputElement>) => setField('port', Number(e.target.value) || 8765)}
                    className={fieldErrors.port ? 'border-red-500' : ''}
                  />
                  {fieldErrors.port && <div className="text-xs text-red-500">{fieldErrors.port}</div>}
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label>Region</Label>
                  <Select value={config.region} onValueChange={(v: string) => setField('region', v || 'us-east-1')}>
                    <SelectTrigger className={fieldErrors.region ? 'border-red-500' : ''}>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="us-east-1">us-east-1</SelectItem>
                      <SelectItem value="us-east-2">us-east-2</SelectItem>
                      <SelectItem value="us-west-1">us-west-1</SelectItem>
                      <SelectItem value="us-west-2">us-west-2</SelectItem>
                      <SelectItem value="eu-central-1">eu-central-1</SelectItem>
                      <SelectItem value="eu-central-2">eu-central-2</SelectItem>
                      <SelectItem value="eu-west-1">eu-west-1</SelectItem>
                      <SelectItem value="eu-west-2">eu-west-2</SelectItem>
                      <SelectItem value="eu-west-3">eu-west-3</SelectItem>
                      <SelectItem value="eu-north-1">eu-north-1</SelectItem>
                      <SelectItem value="eu-south-1">eu-south-1</SelectItem>
                      <SelectItem value="eu-south-2">eu-south-2</SelectItem>
                      <SelectItem value="ap-northeast-1">ap-northeast-1</SelectItem>
                      <SelectItem value="ap-northeast-2">ap-northeast-2</SelectItem>
                      <SelectItem value="ap-northeast-3">ap-northeast-3</SelectItem>
                      <SelectItem value="ap-southeast-1">ap-southeast-1</SelectItem>
                      <SelectItem value="ap-southeast-2">ap-southeast-2</SelectItem>
                      <SelectItem value="ap-southeast-3">ap-southeast-3</SelectItem>
                      <SelectItem value="ap-southeast-4">ap-southeast-4</SelectItem>
                      <SelectItem value="ap-southeast-5">ap-southeast-5</SelectItem>
                      <SelectItem value="ap-southeast-7">ap-southeast-7</SelectItem>
                      <SelectItem value="ap-south-1">ap-south-1</SelectItem>
                      <SelectItem value="ap-south-2">ap-south-2</SelectItem>
                      <SelectItem value="ap-east-1">ap-east-1</SelectItem>
                      <SelectItem value="af-south-1">af-south-1</SelectItem>
                      <SelectItem value="ca-central-1">ca-central-1</SelectItem>
                      <SelectItem value="ca-west-1">ca-west-1</SelectItem>
                      <SelectItem value="sa-east-1">sa-east-1</SelectItem>
                      <SelectItem value="me-south-1">me-south-1</SelectItem>
                      <SelectItem value="me-central-1">me-central-1</SelectItem>
                      <SelectItem value="il-central-1">il-central-1</SelectItem>
                      <SelectItem value="mx-central-1">mx-central-1</SelectItem>
                      <SelectItem value="us-gov-west-1">us-gov-west-1</SelectItem>
                      <SelectItem value="us-gov-east-1">us-gov-east-1</SelectItem>
                      <SelectItem value="cn-north-1">cn-north-1</SelectItem>
                      <SelectItem value="cn-northwest-1">cn-northwest-1</SelectItem>
                    </SelectContent>
                  </Select>
                  {fieldErrors.region && <div className="text-xs text-red-500">{fieldErrors.region}</div>}
                </div>
              </div>
              <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                <div className="flex flex-col gap-1.5">
                  <Label>{t('gateway.accountMode')}</Label>
                  <Select value={config.accountMode} onValueChange={(v: string) => setField('accountMode', v)}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="single">{t('gateway.singleAccount')}</SelectItem>
                      <SelectItem value="group">{t('gateway.byGroup')}</SelectItem>
                      <SelectItem value="pool">{t('gateway.globalPool')}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                {config.accountMode === 'pool' ? (
                  <div className="flex flex-col gap-1.5">
                    <Label>{t('gateway.accountPool')}</Label>
                    <Button
                      variant="outline"
                      className="h-10 justify-start"
                      onClick={() => setShowAccountPoolDialog(true)}
                    >
                      <Users size={16} className="mr-2" />
                      {effectiveSelectedPoolIds.length > 0
                        ? `${t('gateway.selected')} ${effectiveSelectedPoolIds.length} ${t('gateway.accounts')} · ${getStrategyLabel(effectiveStrategy)}`
                        : t('gateway.configureAccountPool')}
                    </Button>
                  </div>
                ) : config.accountMode === 'group' ? (
                  <>
                    <div className="flex flex-col gap-1.5">
                      <Label>{t('gateway.selectGroup')}</Label>
                      <Select value={config.groupId} onValueChange={(v: string) => setField('groupId', v)}>
                        <SelectTrigger className={fieldErrors.groupId ? 'border-red-500' : ''}>
                          <SelectValue placeholder={t('gateway.selectGroupPlaceholder')} />
                        </SelectTrigger>
                        <SelectContent>
                          {groupOptions.map((opt: any) => (
                            <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {fieldErrors.groupId && <div className="text-xs text-red-500">{fieldErrors.groupId}</div>}
                    </div>
                    <div className="flex flex-col gap-1.5">
                      <Label>{t('gateway.routingStrategy')}</Label>
                      <Select value={effectiveStrategy} onValueChange={(v: string) => setField('strategy', v || 'round_robin')}>
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="round_robin"><div className="flex items-center gap-2"><RotateCw size={14} /><span>{t('gateway.roundRobin')}</span></div></SelectItem>
                          <SelectItem value="most_quota"><div className="flex items-center gap-2"><TrendingUp size={14} /><span>{t('gateway.priorityQuota')}</span></div></SelectItem>
                          <SelectItem value="random"><div className="flex items-center gap-2"><Shuffle size={14} /><span>{t('gateway.random')}</span></div></SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                  </>
                ) : (
                  <div className="flex flex-col gap-1.5">
                    <Label>{t('gateway.specifySingleAccount')}</Label>
                    <Select value={config.accountId} onValueChange={(v: string) => setField('accountId', v)}>
                      <SelectTrigger className={fieldErrors.accountId ? 'border-red-500' : ''}>
                        <SelectValue placeholder={t('gateway.selectAGroup')} />
                      </SelectTrigger>
                      <SelectContent>
                        {accountOptions.map((opt: any) => (
                          <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    {fieldErrors.accountId && <div className="text-xs text-red-500">{fieldErrors.accountId}</div>}
                  </div>
                )}
              </div>
            </div>

            {/* Section 2: 客户端认证与模型 */}
            <div className="space-y-3">
              <div className="text-sm font-medium text-foreground flex items-center gap-2">
                <div className="w-1 h-4 bg-primary rounded-full"></div>
                {t('gateway.clientAuthAndModel')}
              </div>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                <div className="flex items-center justify-between p-3 border rounded-lg bg-muted/20">
                  <div className="text-sm text-muted-foreground">
                    {(() => {
                      // 复用共享解析，避免与后端 effective_client_api_keys 语义漂移
                      const totalCount = parseClientApiKeys(config.clientApiKeysText || config.apiKey).length
                      const enabledCount = countEffectiveClientApiKeys(config.clientApiKeysText || config.apiKey)
                      return totalCount > 0
                        ? `${totalCount} ${t('gateway.keys')}, ${enabledCount} ${t('gateway.enabledCount')}`
                        : t('gateway.noApiKey')
                    })()}
                  </div>
                  <Button size="sm" variant="outline" className="h-7 text-sm" onClick={() => setShowApiKeysDialog(true)}>
                    {t('gateway.manageKeys')}
                  </Button>
                </div>
                <div className="flex items-center justify-between p-3 border rounded-lg bg-muted/20">
                  <div className="text-sm text-muted-foreground">
                    {config.modelMappings?.length > 0
                      ? `${config.modelMappings.length} ${t('gateway.mappingRules')}, ${config.modelMappings.filter((r: any) => r.enabled).length} ${t('gateway.enabled')}`
                      : t('gateway.noMappingRules')}
                  </div>
                  <Button size="sm" variant="outline" className="h-7 text-sm" onClick={() => setShowModelMappingDialog(true)}>
                    <Shuffle size={12} className="mr-1" />
                    {t('gateway.mappingRules')}
                  </Button>
                </div>
                {onShowClientConfig && (
                  <div className="flex items-center justify-between p-3 border rounded-lg bg-muted/20">
                    <div className="text-sm text-muted-foreground">
                      {hasConfiguredClients ? t('gateway.clientConfigured') : t('gateway.writeClientConfig')}
                    </div>
                    <Button
                      size="sm"
                      variant={hasConfiguredClients ? "default" : "outline"}
                      className="h-7 text-sm"
                      onClick={onShowClientConfig}
                    >
                      <Zap size={12} className="mr-1" />
                      {hasConfiguredClients ? t('gateway.reconfigure') : t('gateway.configureClient')}
                    </Button>
                  </div>
                )}
              </div>
              {fieldErrors.clientApiKeysText && <div className="text-xs text-red-500">{fieldErrors.clientApiKeysText}</div>}
            </div>

            {/* Section 3: 提示词过滤 */}
            <div className="space-y-3">
              <div className="text-sm font-medium text-foreground flex items-center gap-2">
                <div className="w-1 h-4 bg-primary rounded-full"></div>
                {t('gateway.promptFilter')}
              </div>
              <div className="grid grid-cols-3 gap-2">
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.simplifyCCPrompt')}</Label>
                  <Switch checked={!!config.filterClaudeCode} onCheckedChange={(checked: boolean) => setField('filterClaudeCode', checked)} />
                </div>
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.removeBoundaryMarkers')}</Label>
                  <Switch checked={!!config.filterStripBoundaries} onCheckedChange={(checked: boolean) => setField('filterStripBoundaries', checked)} />
                </div>
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.removeEnvNoise')}</Label>
                  <Switch checked={!!config.filterEnvNoise} onCheckedChange={(checked: boolean) => setField('filterEnvNoise', checked)} />
                </div>
              </div>
              <div className="flex items-center justify-between p-3 border rounded-lg bg-muted/20">
                <div className="text-sm text-muted-foreground">
                  {config.promptFilterRules?.length > 0
                    ? `${config.promptFilterRules.length} ${t('gateway.customRules')}, ${config.promptFilterRules.filter((r: any) => r.enabled).length} ${t('gateway.enabled')}`
                    : t('gateway.noCustomRules')}
                </div>
                <Button size="sm" variant="outline" className="h-7 text-sm" onClick={() => setShowPromptFilterRulesDialog(true)}>
                  {t('gateway.manageRules')}
                </Button>
              </div>
            </div>

            {/* Section 4: 安全与高级 */}
            <div className="space-y-3">
              <div className="text-sm font-medium text-foreground flex items-center gap-2">
                <div className="w-1 h-4 bg-primary rounded-full"></div>
                {t('gateway.securityAndAdvanced')}
              </div>
              <div className="grid grid-cols-3 md:grid-cols-4 gap-2">
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.localOnly')}</Label>
                  <Switch
                    checked={!!config.localOnly}
                    onCheckedChange={(checked: boolean) => {
                      setConfig((prev: any) => applyGatewayLocalOnlyChange(prev, checked, createGeneratedApiKey))
                    }}
                  />
                </div>
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.autoStart')}</Label>
                  <Switch checked={!!config.enabled} onCheckedChange={handleAutoStartToggle} />
                </div>
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.responseCache')}</Label>
                  <Switch checked={!!config.responseCacheEnabled} onCheckedChange={(checked: boolean) => setField('responseCacheEnabled', checked)} />
                </div>
                <div className="flex flex-col gap-0.5 p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-xs text-muted-foreground">{t('gateway.cacheTTLSeconds')}</Label>
                  <Input
                    type="number"
                    value={config.responseCacheTtl}
                    min={30}
                    max={3600}
                    className="h-6 text-sm px-1.5"
                    onChange={(e: React.ChangeEvent<HTMLInputElement>) => setField('responseCacheTtl', Number(e.target.value) || 180)}
                    disabled={!config.responseCacheEnabled}
                  />
                </div>
                <div className="flex flex-col gap-0.5 p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-xs text-muted-foreground">{t('gateway.thresholdPercent')}</Label>
                  <Input
                    type="number"
                    value={config.threshold}
                    min={1}
                    max={100}
                    className="h-6 text-sm px-1.5"
                    onChange={(e: React.ChangeEvent<HTMLInputElement>) => setField('threshold', Number(e.target.value) || 90)}
                  />
                </div>
                <div className="flex items-center justify-between p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-sm">{t('gateway.webSearchEmulation')}</Label>
                  <Switch
                    checked={!!config.webSearchEmulation}
                    onCheckedChange={(checked: boolean) => setField('webSearchEmulation', checked)}
                  />
                </div>
                <div className="flex flex-col gap-0.5 p-2.5 rounded-lg border border-border bg-muted/30">
                  <Label className="text-xs text-muted-foreground">{t('gateway.webSearchTimeout')}</Label>
                  <Input
                    type="number"
                    value={config.webSearchTimeoutSecs}
                    min={5}
                    max={120}
                    className="h-6 text-sm px-1.5"
                    onChange={(e: React.ChangeEvent<HTMLInputElement>) => setField('webSearchTimeoutSecs', Number(e.target.value) || 30)}
                    disabled={!config.webSearchEmulation}
                  />
                </div>
              </div>

              {!config.localOnly && (
                <div className="flex flex-col gap-1.5">
                  <Label>{t('gateway.ipWhitelist')}</Label>
                  <Textarea
                    placeholder={'192.168.1.10\n10.0.0.0/24'}
                    rows={2}
                    value={config.allowedIpsText}
                    onChange={(e: React.ChangeEvent<HTMLTextAreaElement>) => setField('allowedIpsText', e.target.value)}
                    className={fieldErrors.allowedIpsText ? 'border-red-500' : ''}
                  />
                  {fieldErrors.allowedIpsText && <div className="text-xs text-red-500">{fieldErrors.allowedIpsText}</div>}
                </div>
              )}
            </div>
          </div>
        </div>
      </GatewaySurfaceCard>

      {/* AccountPoolDialog */}
      <DialogRoot open={showAccountPoolDialog} onOpenChange={setShowAccountPoolDialog}>
        <DialogContent maxWidth="800px">
          <DialogHeader icon={Users}>
            <DialogTitle>{t('gateway.configureAccountPool')}</DialogTitle>
            <DialogDescription>
              {t('gateway.accountPoolDescription')}
            </DialogDescription>
          </DialogHeader>

          <DialogBody className="space-y-4">
            <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
              <div className="rounded-xl border border-border bg-muted/20 p-3">
                <div className="text-xs text-muted-foreground">{t('gateway.selectedAccounts')}</div>
                <div className="mt-1 text-2xl font-semibold text-foreground">{effectiveSelectedPoolIds.length}<span className="text-sm text-muted-foreground font-normal ml-1">/ {accountOptions.length}</span></div>
              </div>
              <div className="rounded-xl border border-border bg-muted/20 p-3">
                <div className="flex items-center justify-between gap-2">
                  <div className="text-xs text-muted-foreground">{t('gateway.routingStrategy')}</div>
                  {!config.strategy && <Badge variant="outline" className="h-5 rounded-full px-2 text-[10px]">{t('gateway.defaultRoundRobin')}</Badge>}
                </div>
                <Select value={effectiveStrategy} onValueChange={(v: string) => setField('strategy', v || 'round_robin')}>
                  <SelectTrigger className="mt-1 h-9">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="round_robin"><div className="flex items-center gap-2"><RotateCw size={14} /><span>{t('gateway.roundRobin')}</span></div></SelectItem>
                    <SelectItem value="most_quota"><div className="flex items-center gap-2"><TrendingUp size={14} /><span>{t('gateway.priorityQuota')}</span></div></SelectItem>
                    <SelectItem value="random"><div className="flex items-center gap-2"><Shuffle size={14} /><span>{t('gateway.random')}</span></div></SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>

            {/* 搜索 + 过滤 + 操作栏 */}
            <div className="flex items-center gap-2">
              <div className="relative flex-1">
                <Search className="absolute left-2.5 top-2 h-4 w-4 text-muted-foreground" />
                <Input
                  value={poolSearchQuery}
                  onChange={e => setPoolSearchQuery(e.target.value)}
                  placeholder={t('gateway.searchAccounts')}
                  className="h-8 pl-8 text-xs"
                />
              </div>
              <Select value={poolStatusFilter} onValueChange={setPoolStatusFilter}>
                <SelectTrigger className="h-8 w-[100px] text-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">{t('gateway.statusAll')}</SelectItem>
                  <SelectItem value="active">{t('gateway.poolStatusActive')}</SelectItem>
                  <SelectItem value="overage">{t('gateway.poolStatusOverage')}</SelectItem>
                  <SelectItem value="capped">{t('gateway.poolStatusCapped')}</SelectItem>
                  <SelectItem value="banned">{t('gateway.poolStatusBanned')}</SelectItem>
                  <SelectItem value="invalid">{t('gateway.poolStatusInvalid')}</SelectItem>
                </SelectContent>
              </Select>
              <Button type="button" variant="outline" size="sm" className="h-8 text-xs" onClick={toggleAllPoolAccounts} disabled={accountOptions.length === 0}>
                {effectiveSelectedPoolIds.length === accountOptions.length && accountOptions.length > 0 ? t('gateway.deselectAll') : t('gateway.selectAll')}
              </Button>
              <Button type="button" variant="outline" size="sm" className="h-8 text-xs" onClick={() => {
                const activeIds = accountOptions
                  .filter((opt: any) => {
                    const acct = opt.account
                    if (!acct) return false
                    if (acct.enabled === false) return false
                    const status = (acct.status || '').toLowerCase()
                    return status === 'active' || status === 'overage' || status === ''
                  })
                  .map((opt: any) => opt.value)
                setField('poolAccountIds', activeIds)
              }}>
                {t('gateway.selectAvailable')}
              </Button>
            </div>

            {/* 账号列表 */}
            <div className="max-h-[380px] space-y-1.5 overflow-y-auto pr-1">
              {accountOptions.length === 0 ? (
                <div className="rounded-xl border border-dashed border-border p-8 text-center text-sm text-muted-foreground">
                  {t('gateway.noAvailableAccounts')}
                </div>
              ) : (
                filteredPoolAccounts.map((opt: any) => {
                    const checked = selectedPoolAccountIds.includes(opt.value)
                    const acct = opt.account || {}
                    const status = acct.status || 'active'
                    const breakdown = acct.usageData?.usageBreakdownList?.[0]
                    const used = getUsed(acct)
                    const limit = getQuota(acct)
                    const currentOverages = breakdown?.currentOverages ?? 0
                    const overageCap = breakdown?.overageCap ?? 0
                    const percent = limit > 0 ? Math.min(100, (used / limit) * 100) : 0
                    const email = acct.email || acct.userId || t('gateway.unknownAccount')
                    const provider = acct.provider || ''
                    const subPlan = acct.usageData?.subscriptionInfo?.subscriptionTitle || ''
                    const hasData = !!acct.usageData

                    return (
                      <div
                        key={opt.value}
                        role="button"
                        tabIndex={0}
                        className={`flex w-full items-center gap-3 rounded-lg border p-2.5 text-left transition-all cursor-pointer ${checked ? 'border-primary bg-primary/5 ring-1 ring-primary/20' : 'border-border bg-background/70 hover:border-primary/40 hover:bg-primary/5'} ${status === 'banned' || status === 'invalid' ? 'opacity-60' : ''}`}
                        onClick={() => togglePoolAccount(opt.value)}
                        onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') togglePoolAccount(opt.value) }}
                      >
                        <Checkbox checked={checked} onCheckedChange={() => togglePoolAccount(opt.value)} onClick={(e) => e.stopPropagation()} />
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1.5 flex-wrap">
                            <span className="truncate text-xs font-medium text-foreground">{email}</span>
                            {provider && <Badge variant="outline" className="text-[9px] px-1 py-0 h-4 shrink-0">{provider}</Badge>}
                            {subPlan && <Badge variant="secondary" className="text-[9px] px-1 py-0 h-4 shrink-0">{subPlan}</Badge>}
                            <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${POOL_STATUS_COLORS[status] || 'bg-gray-400'}`} />
                            <span className="text-[9px] text-muted-foreground shrink-0">{POOL_STATUS_KEYS[status] ? t(POOL_STATUS_KEYS[status]) : (status || '')}</span>
                          </div>
                          {/* 额度信息 */}
                          {hasData ? (
                            <div className="mt-1.5 flex items-center gap-2">
                              <div className="flex-1 h-1 rounded-full bg-muted overflow-hidden">
                                <div
                                  className={`h-full rounded-full transition-all ${currentOverages > 0 ? 'bg-purple-500' : percent >= 90 ? 'bg-red-500' : percent >= 70 ? 'bg-yellow-500' : 'bg-green-500'}`}
                                  style={{ width: `${Math.min(100, percent)}%` }}
                                />
                              </div>
                              <span className="text-[9px] text-muted-foreground whitespace-nowrap">
                                {currentOverages > 0
                                  ? `⚡${formatUsage(currentOverages)}${overageCap > 0 ? '/' + formatUsage(overageCap) : ''}`
                                  : limit > 0
                                    ? `${formatUsage(Math.max(0, limit - used))}/${formatUsage(limit)} (${Math.round(100 - percent)}%)`
                                    : t('gateway.noQuota')
                                }
                              </span>
                            </div>
                          ) : (
                            <div className="mt-1 text-[9px] text-muted-foreground/60 italic">{t('gateway.notRefreshed')}</div>
                          )}
                        </div>
                        {checked && <CheckCircle2 size={14} className="text-primary shrink-0" />}
                      </div>
                    )
                  })
              )}
            </div>
          </DialogBody>

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowAccountPoolDialog(false)}>{t('gateway.close')}</Button>
            <Button onClick={() => { setShowAccountPoolDialog(false); handleSaveConfig() }}>{t('gateway.saveConfig')}</Button>
          </DialogFooter>
        </DialogContent>
      </DialogRoot>

      {/* ModelMappingDialog */}
      <ModelMappingDialog
        open={showModelMappingDialog}
        onOpenChange={setShowModelMappingDialog}
        modelMappings={config.modelMappings}
        setField={setField}
        onSave={handleSaveConfig}
      />

      {/* ApiKeysDialog */}
      <ApiKeysDialog
        open={showApiKeysDialog}
        onOpenChange={setShowApiKeysDialog}
        clientApiKeysText={config.clientApiKeysText}
        setConfig={setConfig}
        onSave={handleSaveConfig}
      />

      {/* PromptFilterRulesDialog */}
      <PromptFilterRulesDialog
        open={showPromptFilterRulesDialog}
        onOpenChange={setShowPromptFilterRulesDialog}
        promptFilterRules={config.promptFilterRules}
        setField={setField}
        onSave={handleSaveConfig}
      />
    </div>
  )
}

export default GatewayConfig
