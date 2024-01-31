export const onClick = () => {
  const activeLink = document.querySelector('aside ul li a.active')
  activeLink?.setAttribute('data-exit', 'true')

  setTimeout(() => {
    activeLink?.removeAttribute('data-exit')
  }, 300)
}
