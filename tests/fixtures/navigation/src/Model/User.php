<?php
namespace App\Model;

class User extends Entity implements Arrayable
{
    use Timestamps;

    public const ROLE_ADMIN = 'admin';

    private string $name;

    public function __construct(string $name)
    {
        $this->name = $name;
    }

    public function getName(): string
    {
        return $this->name;
    }

    public static function create(string $name): self
    {
        return new static($name);
    }

    public function toArray(): array
    {
        return ['id' => $this->getId(), 'name' => $this->getName()];
    }
}
