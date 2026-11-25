import {
  createContext,
  useContext,
  type AnchorHTMLAttributes,
  type ComponentType,
  type ReactNode,
} from 'react'

export type LinkComponentProps = AnchorHTMLAttributes<HTMLAnchorElement> & {
  href: string
}

export type LinkComponent = ComponentType<LinkComponentProps>

function DefaultLink({ href, ...rest }: LinkComponentProps) {
  return <a href={href} {...rest} />
}

const LinkContext = createContext<LinkComponent>(DefaultLink)

export function LinkProvider({
  component,
  children,
}: {
  component?: LinkComponent
  children: ReactNode
}) {
  return <LinkContext.Provider value={component ?? DefaultLink}>{children}</LinkContext.Provider>
}

export function FluidLink(props: LinkComponentProps) {
  const Component = useContext(LinkContext)
  return <Component {...props} />
}
