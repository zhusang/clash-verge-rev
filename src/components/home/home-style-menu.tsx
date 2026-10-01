import { AutoAwesomeRounded, TuneRounded } from '@mui/icons-material'
import { Button, Menu, MenuItem, Tooltip } from '@mui/material'
import { useCallback, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useVerge } from '@/hooks/use-verge'
import { showNotice } from '@/services/notice-service'

/**
 * 传统首页 header 上的风格切换入口。
 * 选择「简洁风格」后由 `home_style` 驱动首页切换渲染分支。
 */
export const HomeStyleMenu = () => {
  const { t } = useTranslation()
  const { patchVerge } = useVerge()
  const [anchorEl, setAnchorEl] = useState<HTMLElement | null>(null)

  const handleClose = useCallback(() => setAnchorEl(null), [])

  const handleSelect = useCallback(
    async (style: IVergeConfig['home_style']) => {
      handleClose()
      try {
        await patchVerge({ home_style: style })
      } catch (error) {
        showNotice.error(error)
      }
    },
    [handleClose, patchVerge],
  )

  return (
    <>
      <Tooltip title={t('home.minimal.menu.switchToMinimal')}>
        <Button
          variant="text"
          color="inherit"
          size="small"
          onClick={(event) => setAnchorEl(event.currentTarget)}
          startIcon={<TuneRounded />}
          sx={{ fontWeight: 'bold' }}
        >
          {t('home.page.style.traditional')}
        </Button>
      </Tooltip>
      <Menu
        anchorEl={anchorEl}
        open={Boolean(anchorEl)}
        onClose={handleClose}
        transitionDuration={200}
      >
        <MenuItem selected onClick={() => handleSelect('traditional')}>
          {t('home.page.style.traditional')}
        </MenuItem>
        <MenuItem onClick={() => handleSelect('minimal')}>
          <AutoAwesomeRounded fontSize="small" sx={{ mr: 1, opacity: 0.7 }} />
          {t('home.page.style.minimal')}
        </MenuItem>
      </Menu>
    </>
  )
}

export default HomeStyleMenu
