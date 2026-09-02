import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { getCustomModels, setCustomModels } from '@/api/credentials'
import type { CustomModel } from '@/types/api'

export function useCustomModels() {
  return useQuery({
    queryKey: ['custom-models'],
    queryFn: getCustomModels,
    staleTime: 5000,
  })
}

export function useSetCustomModels() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (customModels: CustomModel[]) => setCustomModels({ customModels }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['custom-models'] }),
  })
}
