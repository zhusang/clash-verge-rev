import {
  Button,
  InputAdornment,
  List,
  ListItem,
  ListItemText,
  TextField,
  Typography,
} from '@mui/material'
import { useQueryClient } from '@tanstack/react-query'
import { useLockFn } from 'ahooks'
import type { Ref } from 'react'
import { useImperativeHandle, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BaseDialog, DialogRef } from '@/components/base'
import { useTrafficUsageStatus } from '@/hooks/use-traffic-usage'
import { useVerge } from '@/hooks/use-verge'
import { clearTrafficUsage } from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import parseTraffic from '@/utils/parse-traffic'

const DEFAULT_RETENTION_DAYS = 30
const MIN_RETENTION_DAYS = 1
const MAX_RETENTION_DAYS = 365

const isValidRetention = (days: number) =>
  Number.isInteger(days) &&
  days >= MIN_RETENTION_DAYS &&
  days <= MAX_RETENTION_DAYS

export function TrafficUsageViewer({ ref }: { ref?: Ref<DialogRef> }) {
  const { t } = useTranslation()
  const { verge, patchVerge } = useVerge()
  const queryClient = useQueryClient()

  const [open, setOpen] = useState(false)
  const [confirmOpen, setConfirmOpen] = useState(false)
  const [clearing, setClearing] = useState(false)
  const [retentionDays, setRetentionDays] = useState(DEFAULT_RETENTION_DAYS)

  const { data: status, refetch: refetchStatus } = useTrafficUsageStatus(open)

  useImperativeHandle(ref, () => ({
    open: () => {
      setRetentionDays(
        verge?.traffic_usage_retention_days ?? DEFAULT_RETENTION_DAYS,
      )
      setOpen(true)
    },
    close: () => setOpen(false),
  }))

  const retentionValid = isValidRetention(retentionDays)

  const onSave = useLockFn(async () => {
    if (!retentionValid) {
      showNotice.error('settings.modals.trafficUsage.validation.retentionRange')
      return
    }
    try {
      await patchVerge({ traffic_usage_retention_days: retentionDays })
      setOpen(false)
    } catch (err) {
      showNotice.error(err)
    }
  })

  const onClear = useLockFn(async () => {
    setClearing(true)
    try {
      await clearTrafficUsage()
      await queryClient.invalidateQueries({ queryKey: ['getTrafficUsage'] })
      await refetchStatus()
      showNotice.success('settings.modals.trafficUsage.notifications.cleared')
      setConfirmOpen(false)
    } catch (err) {
      showNotice.error(err)
    } finally {
      setClearing(false)
    }
  })

  return (
    <>
      <BaseDialog
        open={open}
        title={t('settings.modals.trafficUsage.title')}
        contentSx={{ width: 420 }}
        okBtn={t('shared.actions.save')}
        cancelBtn={t('shared.actions.cancel')}
        onClose={() => setOpen(false)}
        onCancel={() => setOpen(false)}
        onOk={onSave}
      >
        <List>
          <ListItem sx={{ padding: '5px 2px' }}>
            <ListItemText
              primary={t('settings.modals.trafficUsage.fields.retentionDays')}
            />
            <TextField
              autoComplete="off"
              size="small"
              type="number"
              sx={{ width: 150 }}
              value={retentionDays}
              error={!retentionValid}
              onChange={(e) => setRetentionDays(Number(e.target.value))}
              slotProps={{
                htmlInput: { min: MIN_RETENTION_DAYS, max: MAX_RETENTION_DAYS },
                input: {
                  endAdornment: (
                    <InputAdornment position="end">
                      {t('settings.modals.trafficUsage.units.days')}
                    </InputAdornment>
                  ),
                },
              }}
            />
          </ListItem>

          {!retentionValid && (
            <ListItem sx={{ padding: '0 2px 5px' }}>
              <Typography variant="caption" color="error">
                {t('settings.modals.trafficUsage.validation.retentionRange')}
              </Typography>
            </ListItem>
          )}

          <ListItem sx={{ padding: '5px 2px' }}>
            <ListItemText
              primary={t('settings.modals.trafficUsage.fields.dbSize')}
            />
            <Typography variant="body2" color="text.secondary">
              {parseTraffic(status?.dbSizeBytes ?? 0).join(' ')}
            </Typography>
          </ListItem>

          <ListItem sx={{ padding: '5px 2px' }}>
            <ListItemText
              primary={t('settings.modals.trafficUsage.actions.clear')}
            />
            <Button
              size="small"
              color="error"
              variant="outlined"
              onClick={() => setConfirmOpen(true)}
            >
              {t('settings.modals.trafficUsage.actions.clear')}
            </Button>
          </ListItem>
        </List>
      </BaseDialog>

      <BaseDialog
        open={confirmOpen}
        title={t('settings.modals.trafficUsage.actions.clear')}
        contentSx={{ width: 380 }}
        okBtn={t('shared.actions.confirm')}
        cancelBtn={t('shared.actions.cancel')}
        loading={clearing}
        onClose={() => setConfirmOpen(false)}
        onCancel={() => setConfirmOpen(false)}
        onOk={onClear}
      >
        <Typography variant="body2">
          {t('settings.modals.trafficUsage.messages.clearConfirm')}
        </Typography>
      </BaseDialog>
    </>
  )
}
