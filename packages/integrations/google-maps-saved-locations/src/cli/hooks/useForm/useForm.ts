import { useMemo, useState } from 'react'

type Value = string
type FormValues = Record<string, Value>
export type Rule<Inputs> = (value: Value, data: Inputs) => string | undefined
type Rules<Inputs> = Record<keyof Inputs, Rule<Inputs>>
type Errors<Inputs> = Partial<Record<keyof Inputs, string>>

interface InputProps<V> {
  onBlur: () => void
  onFocus: () => void
  onChange: (value: V) => void
  value: V
}

type ValidateAction = 'blur' | 'focus' | 'change'

interface FormArgs<Values> {
  initialValues: Values
  rules?: Partial<Rules<Values>>
  validateAction?: ValidateAction | 'never'
}

export const useForm = <Values extends FormValues>({
  initialValues,
  rules = {},
  validateAction = 'blur',
}: FormArgs<Values>) => {
  const [data, setData] = useState(initialValues)
  const [errors, setErrors] = useState<Errors<Values>>({})

  const getIsValid = (errors: Errors<Values>) => {
    return Object.values(errors).every((err) => !err)
  }

  const isValid = useMemo(() => getIsValid(errors), [errors])

  const inputIsValid = (name: keyof Values) => {
    const rule = rules[name]
    return rule ? !rule(data[name], data) : true
  }

  const setInputError = (name: keyof Values, error?: string) => {
    setErrors({
      ...errors,
      [name]: error,
    })
  }

  const validate = () => {
    const newErrors: Errors<Values> = {}
    Object.entries(rules)
      .map((r) => r as [keyof Values, Rule<Values>])
      .forEach(([key, rule]) => {
        newErrors[key] = rule(data[key], data) || ''
      })

    setErrors(newErrors)

    return [getIsValid(newErrors), newErrors] as const
  }

  const validateInput = <T extends keyof Values>(
    name: T,
    newValue?: Values[T],
  ) => {
    const rule = rules[name]
    if (rule) {
      const error = rule(newValue ?? data[name], data) || ''
      if (error !== errors[name]) {
        setErrors((state) => ({ ...state, [name]: error }))
      }
    }
  }

  const validateInputOnAction = <T extends keyof Values>(
    name: T,
    action: ValidateAction,
    newValue?: Values[T],
  ) => {
    if (validateAction === action) {
      validateInput(name, newValue)
    }
  }

  const onChange = <T extends keyof Values>(
    name: T,
    value: Values[T],
    forceValidate = false,
  ) => {
    setData((state) => ({ ...state, [name]: value }))

    if (forceValidate) {
      if (data[name] !== value) {
        validateInput(name, value)
      }
    } else {
      validateInputOnAction(name, 'change', value)
    }
  }

  const onBlur = (name: keyof Values) => {
    validateInputOnAction(name, 'blur')
  }

  const onFocus = (name: keyof Values) => {
    validateInputOnAction(name, 'focus')
  }

  const register = <T extends keyof Values>(name: T): InputProps<Values[T]> => {
    return {
      onBlur: () => onBlur(name),
      onFocus: () => onFocus(name),
      onChange: (value: Values[T]) => onChange(name, value),
      value: data[name] ?? '',
    }
  }

  return {
    register,
    validate,
    validateInput,
    inputIsValid,
    setInputError,
    data,
    errors,
    isValid,
    setData,
    change: onChange,
  }
}

export const combine =
  <T>(...rules: Rule<T>[]) =>
  (value: Value, data: T) => {
    return rules.reduce((err, rule) => {
      return (err || rule(value, data)) as string
    }, '')
  }
