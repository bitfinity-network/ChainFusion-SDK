import { CurrencyAmount, Percent, Token } from 'sdk-core/src/index'
import { useMemo } from 'react'
import { InterfaceTrade } from 'state/routing/types'

export function useMaxAmountIn(trade: InterfaceTrade | undefined, allowedSlippage: Percent) {
  return useMemo(() => {
    const maximumAmountIn = trade?.maximumAmountIn(allowedSlippage)
    return maximumAmountIn?.currency.isToken ? (maximumAmountIn as CurrencyAmount<Token>) : undefined
  }, [allowedSlippage, trade])
}
